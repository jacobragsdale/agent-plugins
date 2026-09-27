using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Xunit;

namespace Marketplace.Api.Tests;

/// <summary>Defects and decisions left open by the live end-to-end audit e2e09270346.</summary>
public sealed partial class MarketplaceApiTests
{
    [Fact]
    public async Task An_install_of_a_package_the_caller_cannot_see_is_refused_like_an_unknown_one()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\hider");
        using var outsider = factory.ClientFor("TEST\\hideoutsider");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("hider", "secret"), "1.0.0"))
        {
            await PublishLiveAsync(owner, "hider", "secret", form);
        }

        await Share(owner, "hider/secret", new { visibility = "private" });
        var now = DateTimeOffset.UtcNow;
        var batch = new
        {
            events = new object[]
            {
                new { kind = "install", occurredAt = now, clientVersion = "0.2.2", packageId = "hider/secret" },
                new { kind = "update", occurredAt = now, clientVersion = "0.2.2", packageId = "hider/secret", toVersion = "1.0.0" },
                new { kind = "install", occurredAt = now, clientVersion = "0.2.2", packageId = "nobody/never" },
                new { kind = "uninstall", occurredAt = now, clientVersion = "0.2.2", packageId = "hider/secret" },
            },
        };
        var response = await outsider.PostAsJsonAsync("/api/events", batch, Json, ct);
        Assert.Equal(HttpStatusCode.Accepted, response.StatusCode);
        var accepted = await response.Content.ReadFromJsonAsync<JsonElement>(Json, ct);
        Assert.Equal(1, accepted.GetProperty("accepted").GetInt32());
        Assert.Equal(3, accepted.GetProperty("rejected").GetInt32());
        Assert.All(accepted.GetProperty("problems").EnumerateArray(), problem => Assert.Contains("never had", problem.GetString()));

        // Nothing counted, so its owners may still delete it.
        Assert.Equal(HttpStatusCode.NoContent, (await owner.DeleteAsync("/api/packages/hider/secret", ct)).StatusCode);
    }

    [Fact]
    public async Task A_pc_that_starts_reporting_its_device_is_counted_once()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\onepcpub");
        using var admin = factory.ClientFor("TEST\\admin");
        using var user = factory.ClientFor("TEST\\onepc");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("onepcpub", "tool"), "1.0.0"))
        {
            await PublishLiveAsync(owner, "onepcpub", "tool", form);
        }

        var old = new { events = new object[] { new { kind = "heartbeat", occurredAt = DateTimeOffset.UtcNow.AddMinutes(-5), clientVersion = "0.1.0", installed = new[] { "onepcpub/tool" } } } };
        Assert.Equal(HttpStatusCode.Accepted, (await user.PostAsJsonAsync("/api/events", old, Json, ct)).StatusCode);
        var updated = new { events = new object[] { new { kind = "heartbeat", occurredAt = DateTimeOffset.UtcNow, clientVersion = "0.2.2", device = "PC-1", installed = new[] { "onepcpub/tool" } } } };
        Assert.Equal(HttpStatusCode.Accepted, (await user.PostAsJsonAsync("/api/events", updated, Json, ct)).StatusCode);

        var installs = await admin.GetFromJsonAsync<JsonElement>("/api/admin/packages/onepcpub/tool/installs", Json, ct);
        Assert.Equal("PC-1", Assert.Single(installs.EnumerateArray()).GetProperty("device").GetString());
    }

    [Fact]
    public async Task An_admin_delete_reaches_the_pcs_that_installed_it_and_a_reused_id_starts_over()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\reuser");
        using var admin = factory.ClientFor("TEST\\admin");
        using var user = factory.ClientFor("TEST\\reuseuser");
        using var bystander = factory.ClientFor("TEST\\bystander");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("reuser", "tool"), "1.0.0"))
        {
            await PublishLiveAsync(owner, "reuser", "tool", form);
        }

        var install = new { events = new object[] { new { kind = "install", occurredAt = DateTimeOffset.UtcNow, clientVersion = "0.2.2", packageId = "reuser/tool" } } };
        Assert.Equal(HttpStatusCode.Accepted, (await user.PostAsJsonAsync("/api/events", install, Json, ct)).StatusCode);
        // A PC that had the old package and never reports again.
        var heartbeat = new { events = new object[] { new { kind = "heartbeat", occurredAt = DateTimeOffset.UtcNow, clientVersion = "0.2.2", device = "PC-GONE", installed = new[] { "reuser/tool" } } } };
        Assert.Equal(HttpStatusCode.Accepted, (await user.PostAsJsonAsync("/api/events", heartbeat, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.Conflict, (await owner.DeleteAsync("/api/packages/reuser/tool", ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await admin.DeleteAsync("/api/packages/reuser/tool", ct)).StatusCode);
        Assert.Contains("reuser/tool", Revoked(await user.GetFromJsonAsync<JsonElement>("/api/index", Json, ct)));
        Assert.DoesNotContain("reuser/tool", Revoked(await bystander.GetFromJsonAsync<JsonElement>("/api/index", Json, ct)));

        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("reuser", "tool", "Something else."), "1.0.0"))
        {
            await PublishLiveAsync(owner, "reuser", "tool", form);
        }

        Assert.DoesNotContain("reuser/tool", Revoked(await user.GetFromJsonAsync<JsonElement>("/api/index", Json, ct)));
        Assert.Equal(0, (await owner.GetFromJsonAsync<JsonElement>("/api/packages/reuser/tool", Json, ct)).GetProperty("installs").GetInt32());
        Assert.Equal(0, (await admin.GetFromJsonAsync<JsonElement>("/api/admin/packages/reuser/tool/installs", Json, ct)).GetArrayLength());
        Assert.Equal(HttpStatusCode.NoContent, (await owner.DeleteAsync("/api/packages/reuser/tool", ct)).StatusCode);
    }

    [Fact]
    public async Task Deleting_a_bundles_last_package_removes_the_bundle()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\kitmaker");
        foreach (var packageId in new[] { "solo", "pair" })
        {
            using var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("kitmaker", packageId), "1.0.0");
            await PublishLiveAsync(owner, "kitmaker", packageId, form);
        }

        Assert.Equal(HttpStatusCode.OK, (await owner.PutAsJsonAsync("/api/bundles/kitmaker/kit", new { name = "Kit", members = new[] { "kitmaker/solo" } }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.OK, (await owner.PutAsJsonAsync("/api/bundles/kitmaker/both", new { name = "Both", members = new[] { "kitmaker/solo", "kitmaker/pair" } }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await owner.DeleteAsync("/api/packages/kitmaker/solo", ct)).StatusCode);

        Assert.Equal(HttpStatusCode.NotFound, (await owner.GetAsync("/api/bundles/kitmaker/kit", ct)).StatusCode);
        var both = await owner.GetFromJsonAsync<JsonElement>("/api/bundles/kitmaker/both", Json, ct);
        Assert.Equal(["kitmaker/pair"], both.GetProperty("members").EnumerateArray().Select(member => member.GetString()).ToArray());
    }

    [Fact]
    public async Task Owners_cannot_delete_a_package_with_an_open_problem_report()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\reported");
        using var reporter = factory.ClientFor("TEST\\reportedby");
        using var admin = factory.ClientFor("TEST\\admin");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("reported", "notes"), "1.0.0"))
        {
            await PublishLiveAsync(owner, "reported", "notes", form);
        }

        Assert.Equal(HttpStatusCode.Accepted, (await reporter.PostAsJsonAsync("/api/packages/reported/notes/reports", new { reason = "It leaks a token." }, Json, ct)).StatusCode);
        var refused = await owner.DeleteAsync("/api/packages/reported/notes", ct);
        Assert.Equal(HttpStatusCode.Conflict, refused.StatusCode);
        Assert.Contains("reported a problem", await Title(refused));

        var report = (await admin.GetFromJsonAsync<JsonElement>("/api/admin/reports", Json, ct)).EnumerateArray().Single(item => item.GetProperty("packageId").GetString() == "reported/notes");
        Assert.Equal(HttpStatusCode.NoContent, (await admin.PostAsJsonAsync($"/api/admin/reports/{report.GetProperty("id").GetInt64()}/resolve", new { note = "Checked." }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await owner.DeleteAsync("/api/packages/reported/notes", ct)).StatusCode);
    }

    [Fact]
    public async Task A_double_clicked_report_is_filed_once()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\twiceowner");
        using var reporter = factory.ClientFor("TEST\\twicereporter");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("twiceowner", "notes"), "1.0.0"))
        {
            await PublishLiveAsync(owner, "twiceowner", "notes", form);
        }

        var body = new { reason = "The dates are wrong.", kind = "feedback" };
        var responses = await Task.WhenAll(
            reporter.PostAsJsonAsync("/api/packages/twiceowner/notes/reports", body, Json, ct),
            reporter.PostAsJsonAsync("/api/packages/twiceowner/notes/reports", body, Json, ct));
        Assert.All(responses, response => Assert.Equal(HttpStatusCode.Accepted, response.StatusCode));
        Assert.Single((await owner.GetFromJsonAsync<JsonElement>("/api/packages/twiceowner/notes/reports", Json, ct)).EnumerateArray());
        Assert.Single(await Notifications(owner), item => item.GetProperty("kind").GetString() == "report.created");

        // Something else to say is a new report.
        Assert.Equal(HttpStatusCode.Accepted, (await reporter.PostAsJsonAsync("/api/packages/twiceowner/notes/reports", new { reason = "And the times.", kind = "feedback" }, Json, ct)).StatusCode);
        Assert.Equal(2, (await owner.GetFromJsonAsync<JsonElement>("/api/packages/twiceowner/notes/reports", Json, ct)).GetArrayLength());
    }

    [Fact]
    public async Task Admins_cannot_be_blocked()
    {
        var ct = TestContext.Current.CancellationToken;
        using var admin = factory.ClientFor("TEST\\admin");
        foreach (var account in new[] { "TEST%5Cadmin", "admin", "admin@test.example" })
        {
            var refused = await admin.PutAsJsonAsync($"/api/admin/blocks/{account}", new { reason = "Oops." }, Json, ct);
            Assert.Equal(HttpStatusCode.UnprocessableEntity, refused.StatusCode);
            Assert.Contains("admins can't be blocked", await Title(refused));
        }

        var blocked = (await admin.GetFromJsonAsync<JsonElement>("/api/admin/blocks", Json, ct)).EnumerateArray().Select(block => block.GetProperty("account").GetString()).ToArray();
        Assert.DoesNotContain("admin", blocked);
        Assert.DoesNotContain("TEST\\admin", blocked);
    }

    [Fact]
    public async Task An_approval_covers_only_the_version_the_admin_reviewed()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\swapper");
        using var admin = factory.ClientFor("TEST\\admin");
        using var stranger = factory.ClientFor("TEST\\swapstranger");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillAndMcpPackage("swapper", "db", "Node server."), "1.0.0"))
        {
            await PublishLiveAsync(owner, "swapper", "db", form);
        }

        // The admin opened the review of 1.0.0; the owner publishes 1.0.1 that runs something else before they approve.
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillAndMcpPackage("swapper", "db", "Shell server.", command: "powershell"), "1.0.1"))
        {
            await PublishLiveAsync(owner, "swapper", "db", form);
        }

        Assert.Equal(HttpStatusCode.UnprocessableEntity, (await admin.PostAsJsonAsync("/api/admin/reviews/swapper/db", new { decision = "approve" }, Json, ct)).StatusCode);
        var stale = await admin.PostAsJsonAsync("/api/admin/reviews/swapper/db", new { decision = "approve", version = "1.0.0" }, Json, ct);
        Assert.Equal(HttpStatusCode.Conflict, stale.StatusCode);
        Assert.Contains("1.0.1 is live now", await Title(stale));
        await AssertHidden(stranger, "swapper", "db");

        Assert.Equal(HttpStatusCode.NoContent, (await admin.PostAsJsonAsync("/api/admin/reviews/swapper/db", new { decision = "approve", version = "1.0.1" }, Json, ct)).StatusCode);
        await AssertVisible(stranger, "swapper", "db");
    }

    [Fact]
    public async Task Notifications_page_back_and_mark_read_only_what_was_shown()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\manynotes");
        using var reader = factory.ClientFor("TEST\\manynotesreader");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("manynotes", "notes"), "1.0.0"))
        {
            await PublishLiveAsync(owner, "manynotes", "notes", form);
        }

        foreach (var reason in new[] { "One.", "Two.", "Three." })
        {
            Assert.Equal(HttpStatusCode.Accepted, (await reader.PostAsJsonAsync("/api/packages/manynotes/notes/reports", new { reason, kind = "feedback" }, Json, ct)).StatusCode);
        }

        static long[] Ids(JsonElement page) => page.GetProperty("items").EnumerateArray().Select(item => item.GetProperty("id").GetInt64()).ToArray();
        var first = Ids(await owner.GetFromJsonAsync<JsonElement>("/api/notifications?limit=2", Json, ct));
        Assert.Equal(2, first.Length);
        var older = Ids(await owner.GetFromJsonAsync<JsonElement>($"/api/notifications?before={first.Min()}&limit=2", Json, ct));
        Assert.True(older.Single() < first.Min());

        Assert.Equal(HttpStatusCode.NoContent, (await owner.PostAsJsonAsync("/api/notifications/read", new { from = first.Min(), upTo = first.Max() }, Json, ct)).StatusCode);
        Assert.Equal(1, (await owner.GetFromJsonAsync<JsonElement>("/api/me", Json, ct)).GetProperty("unreadNotifications").GetInt32());
    }
}
