using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Marketplace.Api.Data;
using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Infrastructure;
using Microsoft.EntityFrameworkCore.Migrations;
using Microsoft.Extensions.DependencyInjection;
using Xunit;

namespace Marketplace.Api.Tests;

/// <summary>Teams anyone creates, public and private spaces, sharing, owner review, revocation, and bundles (ADR 0007).</summary>
public sealed partial class MarketplaceApiTests
{
    [Fact]
    public async Task Anyone_creates_a_team_and_invites_others_to_publish()
    {
        var ct = TestContext.Current.CancellationToken;
        using var founder = factory.ClientFor("TEST\\founder");
        using var joiner = factory.ClientFor("TEST\\joiner");
        var created = await founder.PostAsJsonAsync("/api/teams", new { @namespace = "data-team", displayName = "Data Team" }, Json, ct);
        Assert.Equal(HttpStatusCode.Created, created.StatusCode);
        var team = await created.Content.ReadFromJsonAsync<JsonElement>(Json, ct);
        Assert.Equal("owner", team.GetProperty("role").GetString());
        Assert.Equal("private", team.GetProperty("visibility").GetString());
        Assert.Equal(JsonValueKind.Null, team.GetProperty("invite").ValueKind);

        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("founder", "own"), "1.0.0"))
        {
            await PublishLiveAsync(founder, "founder", "own", form);
        }

        foreach (var (name, status) in new[] { ("data-team", HttpStatusCode.Conflict), ("founder", HttpStatusCode.Conflict), ("official", HttpStatusCode.UnprocessableEntity), ("Bad Name", HttpStatusCode.UnprocessableEntity) })
        {
            Assert.Equal(status, (await joiner.PostAsJsonAsync("/api/teams", new { @namespace = name, displayName = "Other" }, Json, ct)).StatusCode);
        }

        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("data-team", "sql"), "1.0.0"))
        {
            Assert.Equal(HttpStatusCode.Forbidden, (await joiner.PostAsync("/api/packages/data-team/sql/versions", form, ct)).StatusCode);
        }

        var invite = await InviteAsync(founder, "data-team", reset: false);
        Assert.StartsWith("https://marketplace.test/l/", invite);
        Assert.Equal(invite, await InviteAsync(founder, "data-team", reset: false));
        var code = Code(invite);
        var preview = await joiner.GetFromJsonAsync<JsonElement>($"/api/links/{code}", Json, ct);
        Assert.Equal("invite", preview.GetProperty("kind").GetString());
        Assert.Equal("team", preview.GetProperty("targetKind").GetString());
        Assert.Equal("Data Team", preview.GetProperty("name").GetString());
        Assert.Equal("founder", preview.GetProperty("by").GetString());
        Assert.True((await Redeem(joiner, code)).GetProperty("changed").GetBoolean());
        Assert.False((await Redeem(joiner, code)).GetProperty("changed").GetBoolean());

        var me = await joiner.GetFromJsonAsync<JsonElement>("/api/me", Json, ct);
        Assert.Contains("data-team", me.GetProperty("namespaces").EnumerateArray().Select(ns => ns.GetString()));
        var membership = me.GetProperty("teams").EnumerateArray().Single();
        Assert.Equal("Data Team", membership.GetProperty("displayName").GetString());
        Assert.False(membership.GetProperty("owner").GetBoolean());
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("data-team", "sql"), "1.0.0"))
        {
            await PublishLiveAsync(joiner, "data-team", "sql", form);
        }

        var entry = IndexEntry(await joiner.GetFromJsonAsync<JsonElement>("/api/index", Json, ct), "data-team/sql");
        Assert.Equal("team", entry.GetProperty("lane").GetString());
        Assert.Equal("Data Team", entry.GetProperty("publisher").GetProperty("displayName").GetString());
        Assert.True(entry.GetProperty("restricted").GetBoolean());
        Assert.False(entry.GetProperty("sharedWithYou").GetBoolean());

        // Owners add people by name; a bare username matches any domain. Removing them takes publishing away.
        Assert.Equal(HttpStatusCode.Forbidden, (await joiner.PostAsJsonAsync("/api/teams/data-team/members", new { account = "addedbyname" }, Json, ct)).StatusCode);
        var added = await founder.PostAsJsonAsync("/api/teams/data-team/members", new { account = "addedbyname" }, Json, ct);
        Assert.Contains("addedbyname", (await added.Content.ReadFromJsonAsync<JsonElement>(Json, ct)).GetProperty("members").EnumerateArray().Select(member => member.GetProperty("account").GetString()));
        using var byName = factory.ClientFor("OTHER\\addedbyname");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("data-team", "etl"), "1.0.0"))
        {
            await PublishLiveAsync(byName, "data-team", "etl", form);
        }

        Assert.Equal(HttpStatusCode.NoContent, (await founder.DeleteAsync("/api/teams/data-team/members?account=addedbyname", ct)).StatusCode);
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("data-team", "etl"), "1.0.1"))
        {
            Assert.Equal(HttpStatusCode.Forbidden, (await byName.PostAsync("/api/packages/data-team/etl/versions", form, ct)).StatusCode);
        }

        // Joining lets people publish, so only owners hand out or reset the link, and a reset kills the old one.
        Assert.Equal(HttpStatusCode.Forbidden, (await joiner.PostAsJsonAsync("/api/teams/data-team/invite", new { reset = false }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.Forbidden, (await joiner.PostAsJsonAsync("/api/teams/data-team/invite", new { reset = true }, Json, ct)).StatusCode);
        var fresh = await InviteAsync(founder, "data-team", reset: true);
        Assert.NotEqual(invite, fresh);
        var gone = await joiner.GetAsync($"/api/links/{code}", ct);
        Assert.Equal(HttpStatusCode.NotFound, gone.StatusCode);
        Assert.Equal("This link no longer works. Ask the person who sent it for a new one.", await Title(gone));

        // The last owner can't leave, and a team with packages can't be deleted.
        Assert.Equal(HttpStatusCode.Conflict, (await founder.DeleteAsync("/api/teams/data-team/members?account=TEST%5Cfounder", ct)).StatusCode);
        Assert.Equal(HttpStatusCode.Conflict, (await founder.DeleteAsync("/api/teams/data-team", ct)).StatusCode);
        Assert.Equal(HttpStatusCode.OK, (await founder.PostAsJsonAsync("/api/teams/data-team/members", new { account = "TEST\\joiner", owner = true }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await founder.DeleteAsync("/api/teams/data-team/members?account=TEST%5Cfounder", ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NotFound, (await founder.GetAsync("/api/teams/data-team", ct)).StatusCode);
        var teams = await joiner.GetFromJsonAsync<JsonElement>("/api/teams", Json, ct);
        Assert.Equal("owner", teams.EnumerateArray().Single().GetProperty("role").GetString());

        Assert.Equal(HttpStatusCode.Created, (await joiner.PostAsJsonAsync("/api/teams", new { @namespace = "scratch-team", displayName = "Scratch" }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await joiner.DeleteAsync("/api/teams/scratch-team", ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NotFound, (await joiner.GetAsync("/api/teams/scratch-team", ct)).StatusCode);
    }

    [Fact]
    public async Task Private_spaces_hide_everything_but_what_is_made_public_or_shared()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\vaultowner");
        using var stranger = factory.ClientFor("TEST\\vaultstranger");
        Assert.Equal(HttpStatusCode.Created, (await owner.PostAsJsonAsync("/api/teams", new { @namespace = "vault", displayName = "Vault" }, Json, ct)).StatusCode);
        foreach (var packageId in new[] { "inner", "outer" })
        {
            using var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("vault", packageId), "1.0.0");
            await PublishLiveAsync(owner, "vault", packageId, form);
        }

        await AssertHidden(stranger, "vault", "inner");
        var outer = await Share(owner, "vault/outer", new { visibility = "public" });
        Assert.Equal("public", outer.GetProperty("effective").GetString());
        await AssertVisible(stranger, "vault", "outer");
        Assert.Equal(["vault/outer"], IndexIds(await stranger.GetFromJsonAsync<JsonElement>("/api/index", Json, ct), "vault/"));

        // A private skill in a public space.
        using var solo = factory.ClientFor("TEST\\openspace");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("openspace", "secret"), "1.0.0"))
        {
            await PublishLiveAsync(solo, "openspace", "secret", form);
        }

        await Share(solo, "openspace/secret", new { visibility = "private" });
        await AssertHidden(stranger, "openspace", "secret");

        // Shared with a person: a package that follows its private space adds its own list.
        var shared = await Share(owner, "vault/inner", new { visibility = "inherit", users = new[] { "sharee" } });
        Assert.Equal("private", shared.GetProperty("effective").GetString());
        using var sharee = factory.ClientFor("TEST\\sharee");
        var shareeEntry = IndexEntry(await sharee.GetFromJsonAsync<JsonElement>("/api/index", Json, ct), "vault/inner");
        Assert.True(shareeEntry.GetProperty("sharedWithYou").GetBoolean());
        Assert.True(shareeEntry.GetProperty("restricted").GetBoolean());

        // Shared with a team, and with an AD group.
        using var friendLead = factory.ClientFor("TEST\\friendlead");
        Assert.Equal(HttpStatusCode.Created, (await friendLead.PostAsJsonAsync("/api/teams", new { @namespace = "friends", displayName = "Friends", visibility = "public" }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.UnprocessableEntity, (await owner.PutAsJsonAsync("/api/access/vault", new { visibility = "private", teams = new[] { "nobody-team" } }, Json, ct)).StatusCode);
        var space = await Share(owner, "vault", new { visibility = "private", teams = new[] { "friends" } });
        Assert.Equal("Friends", space.GetProperty("teams")[0].GetProperty("displayName").GetString());
        await AssertVisible(friendLead, "vault", "inner");
        await Share(solo, "openspace/secret", new { visibility = "private", groups = new[] { "Auditors" } });
        using var auditor = factory.ClientFor("TEST\\auditor", "auditors");
        await AssertVisible(auditor, "openspace", "secret");

        // Only a team's owners change who sees the team; any member shares a single package.
        Assert.Equal(HttpStatusCode.OK, (await owner.PostAsJsonAsync("/api/teams/vault/members", new { account = "TEST\\vaultmember" }, Json, ct)).StatusCode);
        using var member = factory.ClientFor("TEST\\vaultmember");
        var refused = await member.PutAsJsonAsync("/api/access/vault", new { visibility = "public" }, Json, ct);
        Assert.Equal(HttpStatusCode.Forbidden, refused.StatusCode);
        Assert.Equal("Only the team's owners can change who sees Vault.", await Title(refused));
        Assert.Equal(HttpStatusCode.OK, (await member.PutAsJsonAsync("/api/access/vault/inner", new { visibility = "private" }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.UnprocessableEntity, (await owner.PutAsJsonAsync("/api/access/vault", new { visibility = "inherit" }, Json, ct)).StatusCode);
    }

    [Fact]
    public async Task Share_links_add_whoever_opens_them()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\linker");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("linker", "tool"), "1.0.0"))
        {
            await PublishLiveAsync(owner, "linker", "tool", form);
        }

        await Share(owner, "linker/tool", new { visibility = "private" });
        var link = await ShareLink(owner, "linker/tool", reset: false);
        Assert.Equal(link, (await owner.GetFromJsonAsync<JsonElement>("/api/access/linker/tool", Json, ct)).GetProperty("link").GetString());
        var code = Code(link);

        using var first = factory.ClientFor("TEST\\firstopener");
        await AssertHidden(first, "linker", "tool");
        var preview = await first.GetFromJsonAsync<JsonElement>($"/api/links/{code}", Json, ct);
        Assert.Equal("share", preview.GetProperty("kind").GetString());
        Assert.Equal("package", preview.GetProperty("targetKind").GetString());
        Assert.Equal("tool skill", preview.GetProperty("name").GetString());
        var redeemed = await Redeem(first, code);
        Assert.True(redeemed.GetProperty("changed").GetBoolean());
        Assert.Equal("linker/tool", redeemed.GetProperty("target").GetString());
        Assert.False((await Redeem(first, code)).GetProperty("changed").GetBoolean());
        await AssertVisible(first, "linker", "tool");

        using var forwarded = factory.ClientFor("TEST\\forwardee");
        Assert.True((await Redeem(forwarded, code)).GetProperty("changed").GetBoolean());
        var view = await owner.GetFromJsonAsync<JsonElement>("/api/access/linker/tool", Json, ct);
        Assert.Equal(["TEST\\firstopener", "TEST\\forwardee"], view.GetProperty("users").EnumerateArray().Select(user => user.GetProperty("account").GetString()).ToArray());

        var fresh = await ShareLink(owner, "linker/tool", reset: true);
        Assert.NotEqual(link, fresh);
        Assert.Equal(HttpStatusCode.NotFound, (await first.PostAsync($"/api/links/{code}", null, ct)).StatusCode);
        var space = await owner.PostAsJsonAsync("/api/access/linker/link", new { }, Json, ct);
        var spaceCode = Code((await space.Content.ReadFromJsonAsync<JsonElement>(Json, ct)).GetProperty("link").GetString()!);
        Assert.Equal("space", (await first.GetFromJsonAsync<JsonElement>($"/api/links/{spaceCode}", Json, ct)).GetProperty("targetKind").GetString());
    }

    [Fact]
    public async Task Mcp_servers_wait_for_an_admin_before_the_public_sees_them()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\gated");
        using var stranger = factory.ClientFor("TEST\\gatewatcher");
        using var admin = factory.ClientFor("TEST\\admin");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillAndMcpPackage("gated", "db", "Queries the database."), "1.0.0"))
        {
            Assert.True((await PublishLiveAsync(owner, "gated", "db", form)).GetProperty("waitingForPublicReview").GetBoolean());
        }

        await AssertHidden(stranger, "gated", "db");
        await AssertVisible(owner, "gated", "db");
        Assert.Equal("waiting", (await PublicReview(owner, "gated/db")).GetProperty("state").GetString());
        Assert.Contains("gated/db", await ReviewQueue(admin));
        Assert.Equal(HttpStatusCode.Forbidden, (await owner.GetAsync("/api/admin/reviews", ct)).StatusCode);

        Assert.Equal(HttpStatusCode.UnprocessableEntity, (await admin.PostAsJsonAsync("/api/admin/reviews/gated/db", new { decision = "decline" }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await admin.PostAsJsonAsync("/api/admin/reviews/gated/db", new { decision = "decline", note = "Pin the server's version." }, Json, ct)).StatusCode);
        var declined = await PublicReview(owner, "gated/db");
        Assert.Equal("declined", declined.GetProperty("state").GetString());
        Assert.Equal("Pin the server's version.", declined.GetProperty("note").GetString());
        Assert.DoesNotContain("gated/db", await ReviewQueue(admin));

        using (var form = SamplePackages.PublishForm(SamplePackages.SkillAndMcpPackage("gated", "db", "Queries the database, pinned."), "1.1.0"))
        {
            await PublishLiveAsync(owner, "gated", "db", form);
        }

        Assert.Equal("waiting", (await PublicReview(owner, "gated/db")).GetProperty("state").GetString());
        Assert.Equal(HttpStatusCode.NoContent, (await admin.PostAsJsonAsync("/api/admin/reviews/gated/db", new { decision = "approve" }, Json, ct)).StatusCode);
        await AssertVisible(stranger, "gated", "db");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillAndMcpPackage("gated", "db", "Still the database."), "1.2.0"))
        {
            Assert.False((await PublishLiveAsync(owner, "gated", "db", form)).GetProperty("waitingForPublicReview").GetBoolean());
        }

        await AssertVisible(stranger, "gated", "db");

        // Shared privately it needs nobody; made public, it waits again.
        using var other = factory.ClientFor("TEST\\privmcp");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillAndMcpPackage("privmcp", "tool", "Private tool."), "1.0.0"))
        {
            await PublishLiveAsync(other, "privmcp", "tool", form);
        }

        await Share(other, "privmcp/tool", new { visibility = "private", users = new[] { "TEST\\gatewatcher" } });
        await AssertVisible(stranger, "privmcp", "tool");
        Assert.DoesNotContain("privmcp/tool", await ReviewQueue(admin));
        await Share(other, "privmcp/tool", new { visibility = "public" });
        Assert.Contains("privmcp/tool", await ReviewQueue(admin));
        await AssertHidden(stranger, "privmcp", "tool");
    }

    [Fact]
    public async Task Revoked_packages_leave_the_catalog_and_are_listed_for_removal()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\revoker");
        foreach (var packageId in new[] { "bad", "good" })
        {
            using var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("revoker", packageId), "1.0.0");
            await PublishLiveAsync(owner, "revoker", packageId, form);
        }

        using var stranger = factory.ClientFor("TEST\\revokewatcher");
        await AssertVisible(stranger, "revoker", "bad");
        using (var scope = factory.Services.CreateScope())
        {
            var db = scope.ServiceProvider.GetRequiredService<MarketplaceDbContext>();
            db.Heartbeats.Add(new Heartbeat { Account = "TEST\\lostaccess", OccurredAt = DateTime.UtcNow, ReceivedAt = DateTime.UtcNow, ClientVersion = "0.2.0", OsBuild = "10.0.26100", Installed = ["revoker/bad"], ChecksJson = "{}" });
            await db.SaveChangesAsync(ct);
        }

        await Share(owner, "revoker/bad", new { visibility = "private", users = new[] { "revokewatcher" } });
        Assert.Equal(HttpStatusCode.Forbidden, (await stranger.PutAsync("/api/packages/revoker/bad/revoke", null, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await owner.PutAsync("/api/packages/revoker/bad/revoke", null, ct)).StatusCode);

        var index = await stranger.GetFromJsonAsync<JsonElement>("/api/index", Json, ct);
        Assert.Equal(["revoker/good"], IndexIds(index, "revoker/"));
        Assert.Contains("revoker/bad", Revoked(index));
        Assert.Equal(HttpStatusCode.NotFound, (await stranger.GetAsync("/api/packages/revoker/bad", ct)).StatusCode);
        Assert.Equal(["good"], await ManifestPackageIds(await stranger.GetByteArrayAsync("/api/sources/revoker/archive", ct)));
        using var lost = factory.ClientFor("TEST\\lostaccess");
        Assert.Contains("revoker/bad", Revoked(await lost.GetFromJsonAsync<JsonElement>("/api/index", Json, ct)));
        using var unrelated = factory.ClientFor("TEST\\unrelated");
        Assert.DoesNotContain("revoker/bad", Revoked(await unrelated.GetFromJsonAsync<JsonElement>("/api/index", Json, ct)));
        Assert.True((await owner.GetFromJsonAsync<JsonElement>("/api/packages/revoker/bad", Json, ct)).GetProperty("revoked").GetBoolean());

        Assert.Equal(HttpStatusCode.NoContent, (await owner.DeleteAsync("/api/packages/revoker/bad/revoke", ct)).StatusCode);
        var restored = await stranger.GetFromJsonAsync<JsonElement>("/api/index", Json, ct);
        Assert.DoesNotContain("revoker/bad", Revoked(restored));
        Assert.Contains("revoker/bad", IndexIds(restored, "revoker/"));
    }

    [Fact]
    public async Task Anyone_who_can_see_a_package_suggests_changes_its_owners_decide_on()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\suggestee");
        using var helper = factory.ClientFor("TEST\\helper");
        using var stranger = factory.ClientFor("TEST\\suggestwatcher");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("suggestee", "notes", "Version one."), "1.0.0", "notes"))
        {
            await PublishLiveAsync(owner, "suggestee", "notes", form);
        }

        var suggestion = await Suggest(helper, "suggestee", "notes", SamplePackages.SkillPackage("suggestee", "notes", "Better steps.", extraFile: "tips.md"), "Clearer steps.");
        Assert.Equal("pending", suggestion.GetProperty("state").GetString());
        Assert.Equal("1.0.0", suggestion.GetProperty("baseVersion").GetString());
        Assert.Equal("TEST\\helper", suggestion.GetProperty("suggestedBy").GetString());
        var id = suggestion.GetProperty("id").GetInt64();

        using (var form = SamplePackages.SuggestionForm(SamplePackages.SkillPackage("suggestee", "notes"), "Mine."))
        {
            Assert.Equal(HttpStatusCode.Conflict, (await owner.PostAsync("/api/packages/suggestee/notes/suggestions", form, ct)).StatusCode);
        }

        using (var form = SamplePackages.SuggestionForm(SamplePackages.SkillPackage("suggestee", "notes"), null))
        {
            Assert.Equal(HttpStatusCode.UnprocessableEntity, (await helper.PostAsync("/api/packages/suggestee/notes/suggestions", form, ct)).StatusCode);
        }

        using (var form = SamplePackages.SuggestionForm(SamplePackages.SkillPackage("suggestee", "missing"), "Nope."))
        {
            Assert.Equal(HttpStatusCode.NotFound, (await helper.PostAsync("/api/packages/suggestee/missing/suggestions", form, ct)).StatusCode);
        }

        Assert.Equal(1, (await owner.GetFromJsonAsync<JsonElement>("/api/me", Json, ct)).GetProperty("suggestionsWaiting").GetInt32());
        Assert.Contains((await owner.GetFromJsonAsync<JsonElement>("/api/mine", Json, ct)).GetProperty("suggestions").GetProperty("waiting").EnumerateArray(), item => item.GetProperty("id").GetInt64() == id);
        Assert.Contains((await helper.GetFromJsonAsync<JsonElement>("/api/mine", Json, ct)).GetProperty("suggestions").GetProperty("yours").EnumerateArray(), item => item.GetProperty("id").GetInt64() == id);
        Assert.Single((await helper.GetFromJsonAsync<JsonElement>("/api/packages/suggestee/notes/suggestions", Json, ct)).EnumerateArray());
        Assert.Empty((await stranger.GetFromJsonAsync<JsonElement>("/api/packages/suggestee/notes/suggestions", Json, ct)).EnumerateArray());
        Assert.Equal(HttpStatusCode.NotFound, (await stranger.GetAsync($"/api/suggestions/{id}", ct)).StatusCode);

        var files = (await owner.GetFromJsonAsync<JsonElement>($"/api/suggestions/{id}/files", Json, ct)).EnumerateArray()
            .ToDictionary(file => file.GetProperty("path").GetString()!, file => file.GetProperty("status").GetString());
        Assert.Equal("changed", files["skills/notes/SKILL.md"]);
        Assert.Equal("added", files["skills/notes/tips.md"]);
        Assert.Equal("extra content\n", await owner.GetStringAsync($"/api/suggestions/{id}/files/skills/notes/tips.md", ct));

        Assert.Equal(HttpStatusCode.UnprocessableEntity, (await owner.PostAsJsonAsync($"/api/suggestions/{id}", new { decision = "decline" }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.Forbidden, (await helper.PostAsJsonAsync($"/api/suggestions/{id}", new { decision = "accept" }, Json, ct)).StatusCode);
        var accepted = await owner.PostAsJsonAsync($"/api/suggestions/{id}", new { decision = "accept" }, Json, ct);
        Assert.Equal(HttpStatusCode.OK, accepted.StatusCode);
        var acceptedView = await accepted.Content.ReadFromJsonAsync<JsonElement>(Json, ct);
        Assert.Equal("accepted", acceptedView.GetProperty("state").GetString());
        Assert.Equal("1.0.1", acceptedView.GetProperty("acceptedVersion").GetString());
        var latest = (await stranger.GetFromJsonAsync<JsonElement>("/api/packages/suggestee/notes", Json, ct)).GetProperty("versions")[0];
        Assert.Equal("1.0.1", latest.GetProperty("version").GetString());
        Assert.Equal("TEST\\helper", latest.GetProperty("publishedBy").GetString());
        Assert.Equal("Clearer steps.", latest.GetProperty("changelog").GetString());
        var listed = IndexEntry(await stranger.GetFromJsonAsync<JsonElement>("/api/index", Json, ct), "suggestee/notes");
        Assert.Equal("Better steps.", listed.GetProperty("description").GetString());
        Assert.Equal(["notes"], listed.GetProperty("tags").EnumerateArray().Select(tag => tag.GetString()).ToArray());
        Assert.Equal(HttpStatusCode.Conflict, (await owner.PostAsJsonAsync($"/api/suggestions/{id}", new { decision = "accept" }, Json, ct)).StatusCode);

        // A suggestion made against an older version says so; declining sends a note back.
        var stale = (await Suggest(helper, "suggestee", "notes", SamplePackages.SkillPackage("suggestee", "notes", "Another idea."), "Another idea.")).GetProperty("id").GetInt64();
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("suggestee", "notes", "Version two."), "1.1.0"))
        {
            await PublishLiveAsync(owner, "suggestee", "notes", form);
        }

        var staleView = await helper.GetFromJsonAsync<JsonElement>($"/api/suggestions/{stale}", Json, ct);
        Assert.Equal("1.0.1", staleView.GetProperty("baseVersion").GetString());
        Assert.Equal("1.1.0", staleView.GetProperty("liveVersion").GetString());
        var declined = await (await owner.PostAsJsonAsync($"/api/suggestions/{stale}", new { decision = "decline", note = "Covered by 1.1.0." }, Json, ct)).Content.ReadFromJsonAsync<JsonElement>(Json, ct);
        Assert.Equal("declined", declined.GetProperty("state").GetString());
        Assert.Equal("Covered by 1.1.0.", declined.GetProperty("note").GetString());

        var withdrawn = (await Suggest(helper, "suggestee", "notes", SamplePackages.SkillPackage("suggestee", "notes", "Third idea."), "Third.")).GetProperty("id").GetInt64();
        Assert.Equal(HttpStatusCode.Forbidden, (await owner.DeleteAsync($"/api/suggestions/{withdrawn}", ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await helper.DeleteAsync($"/api/suggestions/{withdrawn}", ct)).StatusCode);
        Assert.Equal("withdrawn", (await helper.GetFromJsonAsync<JsonElement>($"/api/suggestions/{withdrawn}", Json, ct)).GetProperty("state").GetString());
        Assert.Equal(HttpStatusCode.Conflict, (await helper.DeleteAsync($"/api/suggestions/{withdrawn}", ct)).StatusCode);

        if (factory.ValidatorPath is not null)
        {
            var leaky = SamplePackages.Zip(new Dictionary<string, string>
            {
                ["agent-plugins.json"] = """{"version":2,"source":{"id":"suggestee","name":"s","description":"s"},"packages":[{"id":"notes","components":[{"kind":"skill","path":"skills/notes"}]}]}""",
                ["skills/notes/SKILL.md"] = SamplePackages.Skill("notes", "Leaks."),
                ["skills/notes/.env"] = "TOKEN=1\n",
            });
            using var form = SamplePackages.SuggestionForm(leaky, "Adds a token.");
            Assert.Equal(HttpStatusCode.UnprocessableEntity, (await helper.PostAsync("/api/packages/suggestee/notes/suggestions", form, ct)).StatusCode);
        }
    }

    [Fact]
    public async Task Bundles_list_packages_from_anywhere_and_show_each_caller_what_they_can_see()
    {
        var ct = TestContext.Current.CancellationToken;
        using var curator = factory.ClientFor("TEST\\bundler");
        using var other = factory.ClientFor("TEST\\bundleother");
        using var stranger = factory.ClientFor("TEST\\bundlewatcher");
        foreach (var packageId in new[] { "alpha", "beta" })
        {
            using var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("bundler", packageId), "1.0.0");
            await PublishLiveAsync(curator, "bundler", packageId, form);
        }

        foreach (var packageId in new[] { "gamma", "hidden" })
        {
            using var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("bundleother", packageId), "1.0.0");
            await PublishLiveAsync(other, "bundleother", packageId, form);
        }

        await Share(other, "bundleother/hidden", new { visibility = "private" });
        var saved = await curator.PutAsJsonAsync("/api/bundles/bundler/kit", new { name = "Starter kit", members = new[] { "bundler/alpha", "bundleother/gamma" } }, Json, ct);
        Assert.Equal(HttpStatusCode.OK, saved.StatusCode);
        var kit = await saved.Content.ReadFromJsonAsync<JsonElement>(Json, ct);
        Assert.Equal(["bundler/alpha", "bundleother/gamma"], kit.GetProperty("members").EnumerateArray().Select(member => member.GetString()).ToArray());
        Assert.True(kit.GetProperty("owned").GetBoolean());
        Assert.Equal("personal", kit.GetProperty("lane").GetString());

        var unknown = await curator.PutAsJsonAsync("/api/bundles/bundler/kit", new { name = "Kit", members = new[] { "bundleother/hidden", "nope/none" } }, Json, ct);
        Assert.Equal(HttpStatusCode.UnprocessableEntity, unknown.StatusCode);
        Assert.Equal("bundleother/hidden, nope/none are not live packages you can see.", await Title(unknown));
        Assert.Equal(HttpStatusCode.UnprocessableEntity, (await curator.PutAsJsonAsync("/api/bundles/bundler/kit", new { name = "Kit", members = Array.Empty<string>() }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.Conflict, (await curator.PutAsJsonAsync("/api/bundles/bundler/alpha", new { name = "Kit", members = new[] { "bundler/beta" } }, Json, ct)).StatusCode);
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("bundler", "kit"), "1.0.0"))
        {
            Assert.Equal(HttpStatusCode.Conflict, (await curator.PostAsync("/api/packages/bundler/kit/versions", form, ct)).StatusCode);
        }

        Assert.Equal(HttpStatusCode.Forbidden, (await stranger.PutAsJsonAsync("/api/bundles/bundler/kit", new { name = "Mine now", members = new[] { "bundler/alpha" } }, Json, ct)).StatusCode);
        Assert.Equal(["bundler/alpha", "bundleother/gamma"], BundleMembers(await stranger.GetFromJsonAsync<JsonElement>("/api/index", Json, ct), "bundler/kit"));

        // Members follow their own access; a bundle with nothing to show leaves the index.
        await Share(curator, "bundler/alpha", new { visibility = "private" });
        Assert.Equal(["bundleother/gamma"], BundleMembers(await stranger.GetFromJsonAsync<JsonElement>("/api/index", Json, ct), "bundler/kit"));
        Assert.Equal(1, (await stranger.GetFromJsonAsync<JsonElement>("/api/bundles/bundler/kit", Json, ct)).GetProperty("hiddenMembers").GetInt32());
        await Share(other, "bundleother/gamma", new { visibility = "private" });
        Assert.DoesNotContain((await stranger.GetFromJsonAsync<JsonElement>("/api/index", Json, ct)).GetProperty("bundles").EnumerateArray(), bundle => bundle.GetProperty("id").GetString() == "bundler/kit");

        await Share(curator, "bundler/kit", new { visibility = "private" });
        Assert.Equal(HttpStatusCode.NotFound, (await stranger.GetAsync("/api/bundles/bundler/kit", ct)).StatusCode);
        Assert.Contains((await curator.GetFromJsonAsync<JsonElement>("/api/mine", Json, ct)).GetProperty("bundles").EnumerateArray(), bundle => bundle.GetProperty("id").GetString() == "bundler/kit");
        Assert.Equal(HttpStatusCode.NoContent, (await curator.DeleteAsync("/api/bundles/bundler/kit", ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NotFound, (await curator.GetAsync("/api/bundles/bundler/kit", ct)).StatusCode);
    }

    [Fact]
    public async Task Me_reports_the_desktop_app_and_what_it_installed_since()
    {
        var ct = TestContext.Current.CancellationToken;
        using var user = factory.ClientFor("TEST\\appuser");
        Assert.Equal(JsonValueKind.Null, (await user.GetFromJsonAsync<JsonElement>("/api/me", Json, ct)).GetProperty("app").ValueKind);
        foreach (var packageId in new[] { "one", "two" })
        {
            using var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("appuser", packageId), "1.0.0");
            await PublishLiveAsync(user, "appuser", packageId, form);
        }

        var now = DateTimeOffset.UtcNow;
        var batch = new
        {
            events = new object[]
            {
                new { kind = "heartbeat", occurredAt = now.AddMinutes(-10), clientVersion = "0.2.0", osBuild = "10.0.26100", agents = new[] { "cursor" }, installed = new[] { "appuser/one" } },
                new { kind = "install", occurredAt = now.AddMinutes(-5), clientVersion = "0.2.0", packageId = "appuser/two", agents = new[] { "cursor" } },
                new { kind = "uninstall", occurredAt = now.AddMinutes(-1), clientVersion = "0.2.0", packageId = "appuser/one", agents = new[] { "cursor" } },
            },
        };
        Assert.Equal(HttpStatusCode.Accepted, (await user.PostAsJsonAsync("/api/events", batch, Json, ct)).StatusCode);
        var app = (await user.GetFromJsonAsync<JsonElement>("/api/me", Json, ct)).GetProperty("app");
        Assert.Equal("0.2.0", app.GetProperty("version").GetString());
        Assert.Equal("10.0.26100", app.GetProperty("os").GetString());
        Assert.Equal(["appuser/two"], app.GetProperty("installed").EnumerateArray().Select(id => id.GetString()).ToArray());

        using (var scope = factory.Services.CreateScope())
        {
            var db = scope.ServiceProvider.GetRequiredService<MarketplaceDbContext>();
            db.Heartbeats.Add(new Heartbeat { Account = "TEST\\staleapp", OccurredAt = DateTime.UtcNow.AddDays(-40), ReceivedAt = DateTime.UtcNow.AddDays(-40), ClientVersion = "0.1.0", OsBuild = "10.0.26100", ChecksJson = "{}" });
            await db.SaveChangesAsync(ct);
        }

        using var stale = factory.ClientFor("TEST\\staleapp");
        Assert.Equal(JsonValueKind.Null, (await stale.GetFromJsonAsync<JsonElement>("/api/me", Json, ct)).GetProperty("app").ValueKind);
    }

    [Fact]
    public async Task Directory_finds_people_and_the_teams_you_can_see()
    {
        var ct = TestContext.Current.CancellationToken;
        using var dora = factory.ClientFor("TEST\\dora.explorer");
        await dora.GetAsync("/api/me", ct);
        Assert.Equal(HttpStatusCode.Created, (await dora.PostAsJsonAsync("/api/teams", new { @namespace = "explorers", displayName = "Explorers", visibility = "public" }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.Created, (await dora.PostAsJsonAsync("/api/teams", new { @namespace = "hidden-club", displayName = "Hidden explorers" }, Json, ct)).StatusCode);

        using var searcher = factory.ClientFor("TEST\\searcher");
        var people = await searcher.GetFromJsonAsync<JsonElement>("/api/directory?q=DORA", Json, ct);
        var person = people.GetProperty("people").EnumerateArray().Single();
        Assert.Equal("TEST\\dora.explorer", person.GetProperty("account").GetString());
        Assert.Equal("dora.explorer", person.GetProperty("displayName").GetString());
        var teams = (await searcher.GetFromJsonAsync<JsonElement>("/api/directory?q=explor", Json, ct)).GetProperty("teams").EnumerateArray().Select(team => team.GetProperty("namespace").GetString()).ToArray();
        Assert.Equal(["explorers"], teams);
        var mine = (await dora.GetFromJsonAsync<JsonElement>("/api/directory?q=explor", Json, ct)).GetProperty("teams").EnumerateArray().Select(team => team.GetProperty("namespace").GetString()).ToArray();
        Assert.Equal(["explorers", "hidden-club"], mine);
        Assert.Empty((await searcher.GetFromJsonAsync<JsonElement>("/api/directory?q=%25", Json, ct)).GetProperty("people").EnumerateArray());
        Assert.Empty((await searcher.GetFromJsonAsync<JsonElement>("/api/directory", Json, ct)).GetProperty("people").EnumerateArray());
    }

    [Fact]
    public async Task The_self_service_migration_turns_review_decisions_into_liveness_and_approval()
    {
        var ct = TestContext.Current.CancellationToken;
        var options = new DbContextOptionsBuilder<MarketplaceDbContext>().UseNpgsql(await factory.CreateDatabaseAsync("migration_check")).Options;
        await using var db = new MarketplaceDbContext(options);
        var migrator = db.GetService<IMigrator>();
        await migrator.MigrateAsync("20260922181734_AddReviewState", ct);
        await db.Database.ExecuteSqlRawAsync("""
            INSERT INTO "Publishers" ("Namespace", "Account", "DisplayName", "FirstSeenAt", "LastPublishedAt")
              VALUES ('official', 'official', 'Official', now(), now()), ('jacob', 'jacob', 'Jacob', now(), now());
            INSERT INTO "Packages" ("Id", "Namespace", "PackageId", "Name", "Description", "Tags", "CreatedAt", "UpdatedAt")
              VALUES (1, 'jacob', 'db', 'db', 'db', ARRAY[]::text[], now(), now()), (2, 'jacob', 'draft', 'draft', 'draft', ARRAY[]::text[], now(), now());
            INSERT INTO "PackageVersions" ("PackageId", "Version", "StoragePath", "ArchiveDigest", "SizeBytes", "ManifestJson", "ComponentKinds", "Tags", "PublishedBy", "PublishedAt", "Yanked", "ReviewState", "ReviewedBy", "ReviewedAt")
              VALUES (1, '1.0.0', 'a', 'd', 1, 'null', ARRAY['skill', 'mcpServer'], ARRAY[]::text[], 'jacob', now(), false, 'Approved', 'admin', now()),
                     (2, '1.0.0', 'b', 'd', 1, 'null', ARRAY['skill'], ARRAY[]::text[], 'jacob', now(), false, 'Rejected', 'admin', now()),
                     (2, '1.0.1', 'c', 'd', 1, 'null', ARRAY['skill'], ARRAY[]::text[], 'jacob', now(), false, 'Pending', NULL, NULL);
            INSERT INTO "AccessRules" ("Target", "Users", "Groups", "UpdatedBy", "UpdatedAt") VALUES ('jacob', ARRAY['bob'], ARRAY[]::text[], 'jacob', now());
            """, ct);
        await migrator.MigrateAsync(null, ct);

        var publishers = await db.Publishers.ToDictionaryAsync(publisher => publisher.Namespace, publisher => publisher.Kind, ct);
        Assert.Equal(PublisherKind.Official, publishers["official"]);
        Assert.Equal(PublisherKind.Personal, publishers["jacob"]);
        var packages = await db.Packages.Include(package => package.Versions).ToDictionaryAsync(package => package.PackageId, ct);
        Assert.Equal("admin", packages["db"].McpApprovedBy);
        Assert.Null(packages["draft"].McpApprovedBy);
        Assert.Equal([true, false], packages["draft"].Versions.OrderBy(version => version.Version).Select(version => version.Yanked).ToArray());
        Assert.Equal(Visibility.Private, (await db.AccessRules.SingleAsync(ct)).Visibility);
    }

    private static async Task<JsonElement> Share(HttpClient client, string target, object body)
    {
        var response = await client.PutAsJsonAsync($"/api/access/{target}", body, Json, TestContext.Current.CancellationToken);
        Assert.True(response.StatusCode == HttpStatusCode.OK, await response.Content.ReadAsStringAsync(TestContext.Current.CancellationToken));
        return await response.Content.ReadFromJsonAsync<JsonElement>(Json, TestContext.Current.CancellationToken);
    }

    private static async Task<string> ShareLink(HttpClient client, string target, bool reset)
    {
        var response = await client.PostAsJsonAsync($"/api/access/{target}/link", new { reset }, Json, TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
        return (await response.Content.ReadFromJsonAsync<JsonElement>(Json, TestContext.Current.CancellationToken)).GetProperty("link").GetString()!;
    }

    private static async Task<string> InviteAsync(HttpClient client, string ns, bool reset)
    {
        var response = await client.PostAsJsonAsync($"/api/teams/{ns}/invite", new { reset }, Json, TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
        return (await response.Content.ReadFromJsonAsync<JsonElement>(Json, TestContext.Current.CancellationToken)).GetProperty("invite").GetString()!;
    }

    private static async Task<JsonElement> Redeem(HttpClient client, string code)
    {
        var response = await client.PostAsync($"/api/links/{code}", null, TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
        return await response.Content.ReadFromJsonAsync<JsonElement>(Json, TestContext.Current.CancellationToken);
    }

    private static async Task<JsonElement> Suggest(HttpClient client, string ns, string packageId, byte[] archive, string message)
    {
        using var form = SamplePackages.SuggestionForm(archive, message);
        var response = await client.PostAsync($"/api/packages/{ns}/{packageId}/suggestions", form, TestContext.Current.CancellationToken);
        Assert.True(response.StatusCode == HttpStatusCode.Created, await response.Content.ReadAsStringAsync(TestContext.Current.CancellationToken));
        return await response.Content.ReadFromJsonAsync<JsonElement>(Json, TestContext.Current.CancellationToken);
    }

    private static async Task<JsonElement> PublicReview(HttpClient client, string id) =>
        (await client.GetFromJsonAsync<JsonElement>($"/api/packages/{id}", Json, TestContext.Current.CancellationToken)).GetProperty("publicReview");

    private static async Task<string[]> ReviewQueue(HttpClient admin) =>
        (await admin.GetFromJsonAsync<JsonElement>("/api/admin/reviews", Json, TestContext.Current.CancellationToken)).EnumerateArray().Select(review => review.GetProperty("id").GetString()!).ToArray();

    private static string[] Revoked(JsonElement index) =>
        index.GetProperty("revoked").EnumerateArray().Select(id => id.GetString()!).ToArray();

    private static string[] BundleMembers(JsonElement index, string id) =>
        index.GetProperty("bundles").EnumerateArray().Single(bundle => bundle.GetProperty("id").GetString() == id)
            .GetProperty("members").EnumerateArray().Select(member => member.GetString()!).ToArray();

    private static string Code(string link) => link[(link.LastIndexOf('/') + 1)..];
}
