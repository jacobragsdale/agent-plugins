using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Marketplace.Api.Data;
using Microsoft.AspNetCore.Hosting;
using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Infrastructure;
using Microsoft.EntityFrameworkCore.Migrations;
using Xunit;

namespace Marketplace.Api.Tests;

/// <summary>Publishing rules, owner feedback, notifications, the MCP gate's reach, and the admin's fleet and incident tools.</summary>
public sealed partial class MarketplaceApiTests
{
    [Fact]
    public async Task A_retried_publish_is_answered_with_what_was_published()
    {
        var ct = TestContext.Current.CancellationToken;
        using var client = factory.ClientFor("TEST\\idempotent");
        var archive = SamplePackages.SkillPackage("idempotent", "tool");
        using (var form = SamplePackages.PublishForm(archive, "1.0.0"))
        {
            await PublishLiveAsync(client, "idempotent", "tool", form);
        }

        using var retry = SamplePackages.PublishForm(archive, "1.0.0");
        var response = await client.PostAsync("/api/packages/idempotent/tool/versions", retry, ct);
        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
        Assert.Equal("1.0.0", (await response.Content.ReadFromJsonAsync<JsonElement>(Json, ct)).GetProperty("version").GetString());
    }

    [Fact]
    public async Task A_dry_run_checks_everything_and_stores_nothing()
    {
        var ct = TestContext.Current.CancellationToken;
        using var client = factory.ClientFor("TEST\\dryrunner");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("dryrunner", "tool", extraFile: "old.txt"), "1.0.0"))
        {
            await PublishLiveAsync(client, "dryrunner", "tool", form);
        }

        using var dry = SamplePackages.PublishForm(SamplePackages.SkillPackage("dryrunner", "tool", "Changed.", extraFile: "new.txt"), "1.1.0");
        dry.Add(new StringContent("true"), "dryRun");
        var response = await client.PostAsync("/api/packages/dryrunner/tool/versions", dry, ct);
        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
        var view = await response.Content.ReadFromJsonAsync<JsonElement>(Json, ct);
        var files = view.GetProperty("files").EnumerateArray().ToDictionary(file => file.GetProperty("path").GetString()!, file => file.GetProperty("status").GetString());
        Assert.Equal("new", files["skills/tool/new.txt"]);
        Assert.Equal("removed", files["skills/tool/old.txt"]);
        Assert.Equal("changed", files["skills/tool/SKILL.md"]);
        Assert.DoesNotContain("dryrunner/tool/1.1.0.zip", factory.Store.Paths);
        Assert.Equal("1.0.0", IndexEntry(await client.GetFromJsonAsync<JsonElement>("/api/index", Json, ct), "dryrunner/tool").GetProperty("version").GetString());

        // A dry run of a first publish claims nothing either.
        using var fresh = SamplePackages.PublishForm(SamplePackages.SkillPackage("dryrunner", "other"), "1.0.0");
        fresh.Add(new StringContent("true"), "dryRun");
        Assert.Equal(HttpStatusCode.OK, (await client.PostAsync("/api/packages/dryrunner/other/versions", fresh, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NotFound, (await client.GetAsync("/api/packages/dryrunner/other", ct)).StatusCode);
    }

    [Fact]
    public async Task Publishing_can_choose_who_installs_it()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\chooser");
        using var stranger = factory.ClientFor("TEST\\choosewatcher");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("chooser", "draft"), "1.0.0"))
        {
            form.Add(new StringContent("private"), "visibility");
            await PublishLiveAsync(owner, "chooser", "draft", form);
        }

        await AssertHidden(stranger, "chooser", "draft");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("chooser", "draft"), "1.0.1"))
        {
            form.Add(new StringContent("everyone"), "visibility");
            Assert.Equal(HttpStatusCode.UnprocessableEntity, (await owner.PostAsync("/api/packages/chooser/draft/versions", form, ct)).StatusCode);
        }

        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("chooser", "draft"), "1.0.1"))
        {
            form.Add(new StringContent("inherit"), "visibility");
            await PublishLiveAsync(owner, "chooser", "draft", form);
        }

        await AssertVisible(stranger, "chooser", "draft");
    }

    [Fact]
    public async Task A_revoked_package_takes_no_new_versions_and_an_admin_takedown_sticks()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\takedown");
        using var admin = factory.ClientFor("TEST\\admin");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("takedown", "bad"), "1.0.0"))
        {
            await PublishLiveAsync(owner, "takedown", "bad", form);
        }

        Assert.Equal(HttpStatusCode.NoContent, (await admin.PutAsync("/api/packages/takedown/bad/revoke", null, ct)).StatusCode);
        Assert.True((await owner.GetFromJsonAsync<JsonElement>("/api/packages/takedown/bad", Json, ct)).GetProperty("revokedByAdmin").GetBoolean());
        var restore = await owner.DeleteAsync("/api/packages/takedown/bad/revoke", ct);
        Assert.Equal(HttpStatusCode.Forbidden, restore.StatusCode);
        Assert.Contains("only an admin can restore it", await Title(restore));
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("takedown", "bad"), "1.0.1"))
        {
            var response = await owner.PostAsync("/api/packages/takedown/bad/versions", form, ct);
            Assert.Equal(HttpStatusCode.Conflict, response.StatusCode);
            Assert.Contains("An admin removed this package", await Title(response));
        }

        var notice = (await Notifications(owner)).Single(item => item.GetProperty("kind").GetString() == "package.revoked");
        Assert.Equal("/p/takedown/bad", notice.GetProperty("link").GetString());
        Assert.Equal(HttpStatusCode.NoContent, (await admin.DeleteAsync("/api/packages/takedown/bad/revoke", ct)).StatusCode);
        Assert.False((await owner.GetFromJsonAsync<JsonElement>("/api/packages/takedown/bad", Json, ct)).GetProperty("revokedByAdmin").GetBoolean());
    }

    [Fact]
    public async Task A_namespace_too_big_for_the_desktop_app_is_refused()
    {
        var ct = TestContext.Current.CancellationToken;
        using var client = factory.ClientFor("TEST\\bigspace");
        using (var form = SamplePackages.PublishForm(SamplePackages.ManyFilesPackage("bigspace", "first", 1200), "1.0.0"))
        {
            await PublishLiveAsync(client, "bigspace", "first", form);
        }

        using var second = SamplePackages.PublishForm(SamplePackages.ManyFilesPackage("bigspace", "second", 900), "1.0.0");
        var response = await client.PostAsync("/api/packages/bigspace/second/versions", second, ct);
        Assert.Equal(HttpStatusCode.UnprocessableEntity, response.StatusCode);
        Assert.Contains("too big for the desktop app", await Title(response));
        Assert.Equal(HttpStatusCode.NotFound, (await client.GetAsync("/api/packages/bigspace/second", ct)).StatusCode);

        using var single = SamplePackages.PublishForm(SamplePackages.ManyFilesPackage("bigspace", "huge", 2100), "1.0.0");
        var tooMany = await client.PostAsync("/api/packages/bigspace/huge/versions", single, ct);
        Assert.Equal(HttpStatusCode.UnprocessableEntity, tooMany.StatusCode);
        Assert.Contains("more than 2,000 files", await Title(tooMany));
    }

    [Fact]
    public async Task Owners_delete_what_nobody_installed_and_admins_purge_versions()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\deleter");
        using var admin = factory.ClientFor("TEST\\admin");
        using var user = factory.ClientFor("TEST\\deleteuser");
        foreach (var packageId in new[] { "unused", "used" })
        {
            using var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("deleter", packageId), "1.0.0");
            await PublishLiveAsync(owner, "deleter", packageId, form);
        }

        var install = new { events = new object[] { new { kind = "install", occurredAt = DateTimeOffset.UtcNow, clientVersion = "0.2.2", packageId = "deleter/used", agents = new[] { "cursor" } } } };
        Assert.Equal(HttpStatusCode.Accepted, (await user.PostAsJsonAsync("/api/events", install, Json, ct)).StatusCode);

        Assert.Equal(HttpStatusCode.Forbidden, (await user.DeleteAsync("/api/packages/deleter/unused", ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await owner.DeleteAsync("/api/packages/deleter/unused", ct)).StatusCode);
        Assert.DoesNotContain("deleter/unused/1.0.0.zip", factory.Store.Paths);
        // Committed first, so a client that hangs up can't leave the archive behind.
        Assert.False(factory.Store.DeletedCancelably("deleter/unused/1.0.0.zip"));
        // The store keeps the deleted archive's path reserved, so the freed id's 1.0.0 is stored beside it.
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("deleter", "unused", "A new start."), "1.0.0"))
        {
            await PublishLiveAsync(owner, "deleter", "unused", form);
        }

        Assert.Equal(HttpStatusCode.OK, (await owner.GetAsync("/api/packages/deleter/unused/versions/1.0.0/files", ct)).StatusCode);

        var refused = await owner.DeleteAsync("/api/packages/deleter/used", ct);
        Assert.Equal(HttpStatusCode.Conflict, refused.StatusCode);
        Assert.Contains("Remove from every PC", await Title(refused));

        // A purge withdraws the version, deletes its archive, and can't be undone.
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("deleter", "used", "Leaked a token."), "1.1.0"))
        {
            await PublishLiveAsync(owner, "deleter", "used", form);
        }

        Assert.Equal(HttpStatusCode.Forbidden, (await owner.DeleteAsync("/api/admin/packages/deleter/used/versions/1.1.0", ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await admin.DeleteAsync("/api/admin/packages/deleter/used/versions/1.1.0", ct)).StatusCode);
        Assert.DoesNotContain("deleter/used/1.1.0.zip", factory.Store.Paths);
        Assert.False(factory.Store.DeletedCancelably("deleter/used/1.1.0.zip"));
        Assert.Equal("1.0.0", IndexEntry(await user.GetFromJsonAsync<JsonElement>("/api/index", Json, ct), "deleter/used").GetProperty("version").GetString());
        Assert.Equal(HttpStatusCode.NotFound, (await owner.GetAsync("/api/packages/deleter/used/versions/1.1.0/files", ct)).StatusCode);
        Assert.Equal(HttpStatusCode.Conflict, (await owner.DeleteAsync("/api/packages/deleter/used/versions/1.1.0/yank", ct)).StatusCode);
        var detail = await owner.GetFromJsonAsync<JsonElement>("/api/packages/deleter/used", Json, ct);
        Assert.True(detail.GetProperty("versions").EnumerateArray().Single(version => version.GetProperty("version").GetString() == "1.1.0").GetProperty("purged").GetBoolean());
        Assert.Equal(HttpStatusCode.NoContent, (await admin.DeleteAsync("/api/packages/deleter/used", ct)).StatusCode);

        var audit = await admin.GetFromJsonAsync<JsonElement>("/api/admin/audit?limit=1000", Json, ct);
        var actions = audit.EnumerateArray().Where(item => item.GetProperty("target").GetString()!.StartsWith("deleter/", StringComparison.Ordinal)).Select(item => item.GetProperty("action").GetString()).ToArray();
        Assert.Contains("purge", actions);
        Assert.Contains("delete", actions);
        var csv = await admin.GetStringAsync("/api/admin/audit?format=csv&limit=5", ct);
        Assert.StartsWith("id,at,actor,action,target,detail\r\n", csv);
    }

    [Fact]
    public async Task Reports_and_feedback_reach_the_owners_and_their_answer_reaches_the_reporter()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\feedbackowner");
        using var reader = factory.ClientFor("TEST\\feedbackreader");
        using var admin = factory.ClientFor("TEST\\admin");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("feedbackowner", "notes"), "1.0.0"))
        {
            await PublishLiveAsync(owner, "feedbackowner", "notes", form);
        }

        Assert.Equal(HttpStatusCode.UnprocessableEntity, (await reader.PostAsJsonAsync("/api/packages/feedbackowner/notes/reports", new { reason = "Hm.", kind = "rant" }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.Accepted, (await reader.PostAsJsonAsync("/api/packages/feedbackowner/notes/reports", new { reason = "It gets the dates wrong.", kind = "feedback" }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.Accepted, (await reader.PostAsJsonAsync("/api/packages/feedbackowner/notes/reports", new { reason = "It leaks a token." }, Json, ct)).StatusCode);

        Assert.Equal(2, (await owner.GetFromJsonAsync<JsonElement>("/api/me", Json, ct)).GetProperty("reportsWaiting").GetInt32());
        Assert.Equal(HttpStatusCode.Forbidden, (await reader.GetAsync("/api/packages/feedbackowner/notes/reports", ct)).StatusCode);
        var reports = await owner.GetFromJsonAsync<JsonElement>("/api/packages/feedbackowner/notes/reports", Json, ct);
        Assert.Equal(["problem", "feedback"], reports.EnumerateArray().Select(report => report.GetProperty("kind").GetString()).ToArray());
        Assert.Equal(2, (await Notifications(owner)).Count(item => item.GetProperty("kind").GetString() == "report.created"));

        // Admins see only problems.
        var adminReports = (await admin.GetFromJsonAsync<JsonElement>("/api/admin/reports", Json, ct)).EnumerateArray().Where(report => report.GetProperty("packageId").GetString() == "feedbackowner/notes").ToArray();
        Assert.Equal("problem", Assert.Single(adminReports).GetProperty("kind").GetString());

        var feedback = reports.EnumerateArray().Single(report => report.GetProperty("kind").GetString() == "feedback").GetProperty("id").GetInt64();
        Assert.Equal(HttpStatusCode.NotFound, (await reader.PostAsJsonAsync($"/api/reports/{feedback}/resolve", new { note = "Mine now." }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await owner.PostAsJsonAsync($"/api/reports/{feedback}/resolve", new { note = "Fixed in 1.0.1." }, Json, ct)).StatusCode);
        var mine = await reader.GetFromJsonAsync<JsonElement>("/api/reports/mine", Json, ct);
        Assert.Equal("Fixed in 1.0.1.", mine.EnumerateArray().Single(report => report.GetProperty("id").GetInt64() == feedback).GetProperty("note").GetString());
        var resolved = (await Notifications(reader)).Single(item => item.GetProperty("kind").GetString() == "report.resolved");
        Assert.Contains("Fixed in 1.0.1.", resolved.GetProperty("text").GetString());
        Assert.Equal(1, (await owner.GetFromJsonAsync<JsonElement>("/api/me", Json, ct)).GetProperty("reportsWaiting").GetInt32());
    }

    [Fact]
    public async Task Notifications_list_newest_first_page_forward_and_mark_read()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\noticeowner");
        using var helper = factory.ClientFor("TEST\\noticehelper");
        Assert.Equal(HttpStatusCode.Created, (await owner.PostAsJsonAsync("/api/teams", new { @namespace = "notice-team", displayName = "Notice Team", visibility = "public" }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.OK, (await owner.PostAsJsonAsync("/api/teams/notice-team/members", new { account = "TEST\\noticehelper" }, Json, ct)).StatusCode);
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("noticeowner", "tips"), "1.0.0"))
        {
            await PublishLiveAsync(owner, "noticeowner", "tips", form);
        }

        await Share(owner, "noticeowner/tips", new { visibility = "private", users = new[] { "TEST\\noticehelper" } });
        var suggestion = await Suggest(helper, "noticeowner", "tips", SamplePackages.SkillPackage("noticeowner", "tips", "Better tips."), "Clearer wording.");
        Assert.Equal(HttpStatusCode.OK, (await owner.PostAsJsonAsync($"/api/suggestions/{suggestion.GetProperty("id").GetInt64()}", new { decision = "accept" }, Json, ct)).StatusCode);

        var helperItems = await Notifications(helper);
        Assert.Equal(["suggestion.decided", "share.added", "team.added"], helperItems.Select(item => item.GetProperty("kind").GetString()).ToArray());
        Assert.Equal("/teams/notice-team", helperItems[2].GetProperty("link").GetString());
        Assert.Contains("suggestion.created", (await Notifications(owner)).Select(item => item.GetProperty("kind").GetString()));

        var oldest = helperItems[2].GetProperty("id").GetInt64();
        var after = await helper.GetFromJsonAsync<JsonElement>($"/api/notifications?after={oldest}", Json, ct);
        Assert.Equal(["share.added", "suggestion.decided"], after.GetProperty("items").EnumerateArray().Select(item => item.GetProperty("kind").GetString()).ToArray());
        Assert.Equal(3, (await helper.GetFromJsonAsync<JsonElement>("/api/me", Json, ct)).GetProperty("unreadNotifications").GetInt32());
        Assert.Equal(HttpStatusCode.NoContent, (await helper.PostAsJsonAsync("/api/notifications/read", new { upTo = oldest }, Json, ct)).StatusCode);
        Assert.Equal(2, (await helper.GetFromJsonAsync<JsonElement>("/api/me", Json, ct)).GetProperty("unreadNotifications").GetInt32());
        Assert.True((await Notifications(helper))[2].GetProperty("read").GetBoolean());
    }

    [Fact]
    public async Task The_mcp_gate_covers_broad_sharing_and_changed_launch_specs()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\gatewide");
        using var admin = factory.ClientFor("TEST\\admin");
        using var listed = factory.ClientFor("TEST\\gatelisted", "Everyone-Group");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillAndMcpPackage("gatewide", "db", "Queries the database."), "1.0.0"))
        {
            form.Add(new StringContent("private"), "visibility");
            Assert.False((await PublishLiveAsync(owner, "gatewide", "db", form)).GetProperty("waitingForPublicReview").GetBoolean());
        }

        // Shared with a whole AD group, it reaches as many people as public does, so it waits for an admin.
        await Share(owner, "gatewide/db", new { visibility = "private", groups = new[] { "Everyone-Group" } });
        await AssertHidden(listed, "gatewide", "db");
        var review = (await admin.GetFromJsonAsync<JsonElement>("/api/admin/reviews", Json, ct)).EnumerateArray().Single(item => item.GetProperty("id").GetString() == "gatewide/db");
        Assert.Equal("Private: 1 AD group", review.GetProperty("audience").GetString());
        var server = review.GetProperty("mcpServers").EnumerateArray().Single();
        Assert.Equal("node", server.GetProperty("command").GetString());
        Assert.Equal(["DB_TOKEN"], server.GetProperty("envNames").EnumerateArray().Select(name => name.GetString()).ToArray());
        Assert.Equal("stdio", server.GetProperty("transport").GetString());

        // A share link counts the same way.
        await Share(owner, "gatewide/db", new { visibility = "private", users = new[] { "TEST\\gatelisted" } });
        await AssertVisible(listed, "gatewide", "db");
        await ShareLink(owner, "gatewide/db", reset: false);
        await AssertHidden(listed, "gatewide", "db");

        Assert.Equal(HttpStatusCode.NoContent, (await admin.PostAsJsonAsync("/api/admin/reviews/gatewide/db", new { decision = "approve", version = "1.0.0" }, Json, ct)).StatusCode);
        await AssertVisible(listed, "gatewide", "db");
        Assert.Contains("review.decided", (await Notifications(owner)).Select(item => item.GetProperty("kind").GetString()));

        // The same server in a new version stays approved; a different command waits again.
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillAndMcpPackage("gatewide", "db", "Same server."), "1.1.0"))
        {
            Assert.False((await PublishLiveAsync(owner, "gatewide", "db", form)).GetProperty("waitingForPublicReview").GetBoolean());
        }

        using (var form = SamplePackages.PublishForm(SamplePackages.SkillAndMcpPackage("gatewide", "db", "New server.", command: "python"), "1.2.0"))
        {
            var published = await PublishLiveAsync(owner, "gatewide", "db", form);
            Assert.True(published.GetProperty("waitingForPublicReview").GetBoolean());
            Assert.Contains("approve it again", published.GetProperty("warnings")[0].GetString());
        }

        await AssertHidden(listed, "gatewide", "db");
        var entry = IndexEntry(await owner.GetFromJsonAsync<JsonElement>("/api/index", Json, ct), "gatewide/db");
        Assert.False(entry.GetProperty("mcpApproved").GetBoolean());
        Assert.Equal(["stdio"], entry.GetProperty("mcpTransports").EnumerateArray().Select(transport => transport.GetString()).ToArray());
        Assert.Equal("python", (await owner.GetFromJsonAsync<JsonElement>("/api/packages/gatewide/db", Json, ct)).GetProperty("mcpServers")[0].GetProperty("command").GetString());
    }

    [Fact]
    public async Task Adding_an_mcp_server_to_a_public_package_warns_who_stops_seeing_it()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\growsmcp");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("growsmcp", "db"), "1.0.0"))
        {
            Assert.Empty((await PublishLiveAsync(owner, "growsmcp", "db", form)).GetProperty("warnings").EnumerateArray());
        }

        using var withServer = SamplePackages.PublishForm(SamplePackages.SkillAndMcpPackage("growsmcp", "db", "Now with a server."), "1.1.0");
        var published = await PublishLiveAsync(owner, "growsmcp", "db", withServer);
        Assert.Contains("PCs that have it keep the version they have", published.GetProperty("warnings")[0].GetString());
        Assert.Null(IndexEntry(await owner.GetFromJsonAsync<JsonElement>("/api/index", Json, ct), "growsmcp/db").GetProperty("changelog").GetString());
    }

    [Fact]
    public async Task Old_desktop_apps_are_told_to_update()
    {
        var ct = TestContext.Current.CancellationToken;
        using var client = factory.ClientFor("TEST\\oldapp");
        client.DefaultRequestHeaders.UserAgent.ParseAdd("agent-plugins/0.0.9");
        var response = await client.GetAsync("/api/catalog", ct);
        Assert.Equal(HttpStatusCode.UpgradeRequired, response.StatusCode);
        var problem = await response.Content.ReadFromJsonAsync<JsonElement>(Json, ct);
        Assert.Equal("Update Agent Plugins", problem.GetProperty("title").GetString());
        Assert.Equal("https://marketplace.test/#download", problem.GetProperty("downloadUrl").GetString());
        Assert.Contains("0.0.9", problem.GetProperty("detail").GetString());

        // It can still say who it is and report, so the admin sees it.
        Assert.Equal(HttpStatusCode.OK, (await client.GetAsync("/api/me", ct)).StatusCode);
        var heartbeat = new { events = new object[] { new { kind = "heartbeat", occurredAt = DateTimeOffset.UtcNow, clientVersion = "0.0.9" } } };
        Assert.Equal(HttpStatusCode.Accepted, (await client.PostAsJsonAsync("/api/events", heartbeat, Json, ct)).StatusCode);

        using var current = factory.ClientFor("TEST\\newapp");
        current.DefaultRequestHeaders.UserAgent.ParseAdd("agent-plugins/0.3.0");
        Assert.Equal(HttpStatusCode.OK, (await current.GetAsync("/api/catalog", ct)).StatusCode);
    }

    [Fact]
    public async Task Admins_see_who_has_a_package_on_which_pc()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\fleetpub");
        using var admin = factory.ClientFor("TEST\\admin");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("fleetpub", "widely"), "1.0.0"))
        {
            await PublishLiveAsync(owner, "fleetpub", "widely", form);
        }

        using var user = factory.ClientFor("TEST\\twopcs");
        var now = DateTimeOffset.UtcNow;
        var batch = new
        {
            events = new object[]
            {
                new { kind = "heartbeat", occurredAt = now.AddMinutes(-2), clientVersion = "0.2.2", device = "LAPTOP-1", installed = new[] { "fleetpub/widely" }, installedVersions = new Dictionary<string, string> { ["fleetpub/widely"] = "1.0.0" } },
                new { kind = "heartbeat", occurredAt = now.AddMinutes(-1), clientVersion = "0.2.2", device = "VDI-7", installed = new[] { "fleetpub/widely" } },
                new { kind = "install", occurredAt = now.AddMinutes(-3), clientVersion = "0.2.2", packageId = "fleetpub/widely" },
                new { kind = "install", occurredAt = now.AddMinutes(-4), clientVersion = "0.2.2", packageId = "fleetpub/widely" },
                new { kind = "install", occurredAt = now.AddMinutes(-4), clientVersion = "0.2.2", packageId = "nobody/never" },
            },
        };
        var accepted = await (await user.PostAsJsonAsync("/api/events", batch, Json, ct)).Content.ReadFromJsonAsync<JsonElement>(Json, ct);
        Assert.Equal(1, accepted.GetProperty("rejected").GetInt32());
        Assert.Contains("never had", accepted.GetProperty("problems")[0].GetString());

        var installs = await admin.GetFromJsonAsync<JsonElement>("/api/admin/packages/fleetpub/widely/installs", Json, ct);
        Assert.Equal(["VDI-7", "LAPTOP-1"], installs.EnumerateArray().Select(install => install.GetProperty("device").GetString()).ToArray());
        Assert.Equal("1.0.0", installs[1].GetProperty("version").GetString());
        Assert.StartsWith("account,device,version,clientVersion,lastSeenAt\r\nTEST\\twopcs,VDI-7,,0.2.2,", await admin.GetStringAsync("/api/admin/packages/fleetpub/widely/installs?format=csv", ct));
        Assert.Equal(HttpStatusCode.Forbidden, (await owner.GetAsync("/api/admin/packages/fleetpub/widely/installs", ct)).StatusCode);

        // Two installs and two PCs from one person count once.
        var detail = await owner.GetFromJsonAsync<JsonElement>("/api/packages/fleetpub/widely", Json, ct);
        Assert.Equal(1, detail.GetProperty("installs").GetInt32());
        Assert.Equal(1, detail.GetProperty("installedBase").GetInt32());
        Assert.Equal("personal", detail.GetProperty("lane").GetString());
        Assert.Equal("fleetpub", detail.GetProperty("publisher").GetProperty("displayName").GetString());
    }

    [Fact]
    public async Task A_blocked_account_can_read_but_not_change_anything_and_its_space_is_hidden()
    {
        var ct = TestContext.Current.CancellationToken;
        using var blocked = factory.ClientFor("TEST\\troublemaker");
        using var admin = factory.ClientFor("TEST\\admin");
        using var stranger = factory.ClientFor("TEST\\bystander");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("troublemaker", "spam"), "1.0.0"))
        {
            await PublishLiveAsync(blocked, "troublemaker", "spam", form);
        }

        Assert.Equal(HttpStatusCode.NoContent, (await admin.PutAsJsonAsync("/api/admin/blocks/TEST%5Ctroublemaker", new { reason = "Spam." }, Json, ct)).StatusCode);
        Assert.Equal("Spam.", (await admin.GetFromJsonAsync<JsonElement>("/api/admin/blocks", Json, ct)).EnumerateArray().Single(block => block.GetProperty("account").GetString() == "TEST\\troublemaker").GetProperty("reason").GetString());
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("troublemaker", "spam"), "1.0.1"))
        {
            var refused = await blocked.PostAsync("/api/packages/troublemaker/spam/versions", form, ct);
            Assert.Equal(HttpStatusCode.Forbidden, refused.StatusCode);
            Assert.Contains("blocked", await Title(refused));
        }

        Assert.Equal(HttpStatusCode.OK, (await blocked.GetAsync("/api/me", ct)).StatusCode);
        await AssertHidden(stranger, "troublemaker", "spam");
        await AssertVisible(admin, "troublemaker", "spam");

        Assert.Equal(HttpStatusCode.NoContent, (await admin.DeleteAsync("/api/admin/blocks/TEST%5Ctroublemaker", ct)).StatusCode);
        await AssertVisible(stranger, "troublemaker", "spam");
    }

    [Fact]
    public async Task An_admin_hands_a_departed_owners_space_to_a_colleague_as_a_team()
    {
        var ct = TestContext.Current.CancellationToken;
        using var departed = factory.ClientFor("TEST\\departed");
        using var admin = factory.ClientFor("TEST\\admin");
        using var heir = factory.ClientFor("TEST\\heir");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("departed", "legacy"), "1.0.0"))
        {
            await PublishLiveAsync(departed, "departed", "legacy", form);
        }

        Assert.Equal(HttpStatusCode.NotFound, (await admin.PutAsJsonAsync("/api/admin/namespaces/nobody-here/owner", new { account = "TEST\\heir" }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await admin.PutAsJsonAsync("/api/admin/namespaces/departed/owner", new { account = "TEST\\heir" }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.Conflict, (await admin.PutAsJsonAsync("/api/admin/namespaces/departed/owner", new { account = "TEST\\heir" }, Json, ct)).StatusCode);
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("departed", "legacy"), "1.0.1"))
        {
            await PublishLiveAsync(heir, "departed", "legacy", form);
        }

        Assert.Equal("owner", (await heir.GetFromJsonAsync<JsonElement>("/api/teams/departed", Json, ct)).GetProperty("role").GetString());
        Assert.Equal("u-departed", (await departed.GetFromJsonAsync<JsonElement>("/api/me", Json, ct)).GetProperty("namespace").GetString());
    }

    [Fact]
    public async Task An_admin_deletes_a_team_with_packages_by_removing_them_from_every_pc()
    {
        var ct = TestContext.Current.CancellationToken;
        using var lead = factory.ClientFor("TEST\\teamlead");
        using var admin = factory.ClientFor("TEST\\admin");
        Assert.Equal(HttpStatusCode.Created, (await lead.PostAsJsonAsync("/api/teams", new { @namespace = "doomed", displayName = "Doomed", visibility = "public" }, Json, ct)).StatusCode);
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("doomed", "tool"), "1.0.0"))
        {
            await PublishLiveAsync(lead, "doomed", "tool", form);
        }

        using var user = factory.ClientFor("TEST\\doomeduser");
        using var bystander = factory.ClientFor("TEST\\bystander");
        var heartbeat = new { events = new object[] { new { kind = "heartbeat", occurredAt = DateTimeOffset.UtcNow, clientVersion = "0.2.2", installed = new[] { "doomed/tool" } } } };
        Assert.Equal(HttpStatusCode.Accepted, (await user.PostAsJsonAsync("/api/events", heartbeat, Json, ct)).StatusCode);

        Assert.Equal(HttpStatusCode.Conflict, (await lead.DeleteAsync("/api/teams/doomed", ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await admin.DeleteAsync("/api/teams/doomed", ct)).StatusCode);
        // The PC that has it gets the removal; the name stays private, so nobody else learns the id.
        Assert.Contains("doomed/tool", Revoked(await user.GetFromJsonAsync<JsonElement>("/api/index", Json, ct)));
        Assert.DoesNotContain("doomed/tool", Revoked(await bystander.GetFromJsonAsync<JsonElement>("/api/index", Json, ct)));
        Assert.DoesNotContain("doomed", (await lead.GetFromJsonAsync<JsonElement>("/api/me", Json, ct)).GetProperty("namespaces").EnumerateArray().Select(ns => ns.GetString()));
        Assert.Equal(HttpStatusCode.Conflict, (await lead.PostAsJsonAsync("/api/teams", new { @namespace = "doomed", displayName = "Again" }, Json, ct)).StatusCode);
    }

    [Fact]
    public async Task Admin_accounts_match_their_domain()
    {
        using var impostor = factory.ClientFor("OTHER\\admin");
        Assert.Equal(HttpStatusCode.Forbidden, (await impostor.GetAsync("/api/admin/summary", TestContext.Current.CancellationToken)).StatusCode);
    }

    [Fact]
    public async Task The_index_and_catalog_let_clients_skip_what_has_not_changed()
    {
        var ct = TestContext.Current.CancellationToken;
        using var client = factory.ClientFor("TEST\\etagger");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("etagger", "tool"), "1.0.0", changelog: "First."))
        {
            await PublishLiveAsync(client, "etagger", "tool", form);
        }

        var first = await client.GetAsync("/api/index", ct);
        var etag = first.Headers.ETag!.Tag;
        using (var conditional = new HttpRequestMessage(HttpMethod.Get, "/api/index"))
        {
            conditional.Headers.IfNoneMatch.ParseAdd(etag);
            Assert.Equal(HttpStatusCode.NotModified, (await client.SendAsync(conditional, ct)).StatusCode);
        }

        var entry = IndexEntry(await first.Content.ReadFromJsonAsync<JsonElement>(Json, ct), "etagger/tool");
        Assert.Equal("First.", entry.GetProperty("changelog").GetString());
        Assert.Equal(JsonValueKind.Null, entry.GetProperty("mcpApproved").ValueKind);
        Assert.Empty(entry.GetProperty("mcpTransports").EnumerateArray());
        Assert.NotEqual(default, entry.GetProperty("createdAt").GetDateTime());

        var source = (await client.GetFromJsonAsync<JsonElement>("/api/catalog", Json, ct)).GetProperty("sources").EnumerateArray().Single(candidate => candidate.GetProperty("sourceId").GetString() == "etagger");
        var archive = await client.GetAsync("/api/sources/etagger/archive", ct);
        Assert.Equal($"\"{source.GetProperty("digest").GetString()}\"", archive.Headers.ETag!.Tag);

        using var compressed = new HttpRequestMessage(HttpMethod.Get, "/api/index");
        compressed.Headers.AcceptEncoding.ParseAdd("gzip");
        Assert.Contains("gzip", (await client.SendAsync(compressed, ct)).Content.Headers.ContentEncoding);
    }

    [Fact]
    public async Task Health_reports_the_package_store_and_ldap()
    {
        using var client = factory.CreateClient();
        var health = await client.GetFromJsonAsync<JsonElement>("/api/health", Json, TestContext.Current.CancellationToken);
        Assert.Equal("ok", health.GetProperty("artifactStore").GetString());
        Assert.Equal("off", health.GetProperty("ldap").GetString());
    }

    [Fact]
    public async Task Too_many_uploads_in_an_hour_are_refused_with_when_to_retry()
    {
        var ct = TestContext.Current.CancellationToken;
        using var limited = factory.WithWebHostBuilder(builder => builder.UseSetting("RateLimits:UploadsPerHour", "1"));
        using var client = limited.CreateClient();
        client.DefaultRequestHeaders.Add("X-Dev-User", "TEST\\hurried");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("hurried", "one"), "1.0.0"))
        {
            await PublishLiveAsync(client, "hurried", "one", form);
        }

        using var second = SamplePackages.PublishForm(SamplePackages.SkillPackage("hurried", "two"), "1.0.0");
        var response = await client.PostAsync("/api/packages/hurried/two/versions", second, ct);
        Assert.Equal(HttpStatusCode.TooManyRequests, response.StatusCode);
        Assert.NotNull(response.Headers.RetryAfter);
        Assert.StartsWith("Too many changes", await Title(response));
    }

    [Fact]
    public async Task The_fleet_migration_keeps_heartbeats_and_reports()
    {
        var ct = TestContext.Current.CancellationToken;
        var options = new DbContextOptionsBuilder<MarketplaceDbContext>().UseNpgsql(await factory.CreateDatabaseAsync("fleet_migration_check")).Options;
        await using var db = new MarketplaceDbContext(options);
        var migrator = db.GetService<IMigrator>();
        await migrator.MigrateAsync("20260926172514_SelfService", ct);
        await db.Database.ExecuteSqlRawAsync("""
            INSERT INTO "Heartbeats" ("Account", "OccurredAt", "ClientVersion", "OsBuild", "Agents", "Installed", "ChecksJson", "ReceivedAt")
              VALUES ('CORP\jane', now(), '0.2.2', '10.0.26100', ARRAY['cursor'], ARRAY['jane/tool'], '{{}}', now());
            INSERT INTO "Reports" ("Account", "PackageId", "Reason", "CreatedAt") VALUES ('CORPob', 'jane/tool', 'Broken.', now());
            """, ct);
        await migrator.MigrateAsync(null, ct);

        var heartbeat = await db.Heartbeats.SingleAsync(ct);
        Assert.Equal((string.Empty, "{}"), (heartbeat.Device, heartbeat.InstalledVersionsJson));
        Assert.Equal(PackageReport.Problem, (await db.Reports.SingleAsync(ct)).Kind);
    }

    private static async Task<JsonElement[]> Notifications(HttpClient client) =>
        (await client.GetFromJsonAsync<JsonElement>("/api/notifications", Json, TestContext.Current.CancellationToken)).GetProperty("items").EnumerateArray().ToArray();
}
