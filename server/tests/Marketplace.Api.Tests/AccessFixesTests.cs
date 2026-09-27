using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Xunit;

namespace Marketplace.Api.Tests;

/// <summary>Defects found by the live end-to-end audit of c821355.</summary>
public sealed partial class MarketplaceApiTests
{
    [Fact]
    public async Task An_anonymous_admin_request_is_a_401_not_a_500()
    {
        using var anonymous = factory.CreateClient();
        var response = await anonymous.GetAsync("/api/admin/summary", TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.Unauthorized, response.StatusCode);
    }

    [Fact]
    public async Task Only_those_who_may_set_a_list_get_its_share_link()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\linkowner");
        using var member = factory.ClientFor("TEST\\linkmember");
        using var outsider = factory.ClientFor("TEST\\linkoutsider");
        Assert.Equal(HttpStatusCode.Created, (await owner.PostAsJsonAsync("/api/teams", new { @namespace = "link-team", displayName = "Link Team" }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.OK, (await owner.PostAsJsonAsync("/api/teams/link-team/members", new { account = "TEST\\linkmember", owner = false }, Json, ct)).StatusCode);
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("link-team", "notes"), "1.0.0"))
        {
            await PublishLiveAsync(member, "link-team", "notes", form);
        }

        // The team's space is its owners' to share; a package in it is any member's.
        Assert.Equal(HttpStatusCode.Forbidden, (await member.PostAsJsonAsync("/api/access/link-team/link", new { reset = false }, Json, ct)).StatusCode);
        Assert.StartsWith("https://marketplace.test/l/", await ShareLink(member, "link-team/notes", reset: false));
        Assert.StartsWith("https://marketplace.test/l/", await ShareLink(owner, "link-team", reset: false));

        // Nobody may share a personal namespace before claiming it by publishing.
        Assert.Equal(HttpStatusCode.Conflict, (await outsider.PostAsJsonAsync("/api/access/linkoutsider/link", new { reset = false }, Json, ct)).StatusCode);
    }

    [Fact]
    public async Task A_link_redeemed_while_the_owner_is_blocked_leaves_the_space_public()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\blockedsharer");
        using var redeemer = factory.ClientFor("TEST\\blockredeemer");
        using var viewer = factory.ClientFor("TEST\\blockviewer");
        using var admin = factory.ClientFor("TEST\\admin");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("blockedsharer", "tips"), "1.0.0"))
        {
            await PublishLiveAsync(owner, "blockedsharer", "tips", form);
        }

        var code = Code(await ShareLink(owner, "blockedsharer", reset: false));
        Assert.Equal(HttpStatusCode.NoContent, (await admin.PutAsJsonAsync("/api/admin/blocks/TEST%5Cblockedsharer", new { reason = "Audit." }, Json, ct)).StatusCode);
        await Redeem(redeemer, code);
        Assert.Equal(HttpStatusCode.NoContent, (await admin.DeleteAsync("/api/admin/blocks/TEST%5Cblockedsharer", ct)).StatusCode);

        Assert.Equal(HttpStatusCode.OK, (await viewer.GetAsync("/api/packages/blockedsharer/tips", ct)).StatusCode);
    }

    [Fact]
    public async Task An_admin_revoke_takes_over_an_owner_revoke()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\revoker");
        using var admin = factory.ClientFor("TEST\\admin");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("revoker", "leak"), "1.0.0"))
        {
            await PublishLiveAsync(owner, "revoker", "leak", form);
        }

        Assert.Equal(HttpStatusCode.NoContent, (await owner.PutAsync("/api/packages/revoker/leak/revoke", null, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await admin.PutAsync("/api/packages/revoker/leak/revoke", null, ct)).StatusCode);

        Assert.True((await owner.GetFromJsonAsync<JsonElement>("/api/packages/revoker/leak", Json, ct)).GetProperty("revokedByAdmin").GetBoolean());
        Assert.Equal(HttpStatusCode.Forbidden, (await owner.DeleteAsync("/api/packages/revoker/leak/revoke", ct)).StatusCode);
    }

    [Fact]
    public async Task A_kerberos_caller_is_the_member_its_domain_entry_names()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\realmowner");
        using var kerberos = factory.ClientFor("realmjane@TEST.EXAMPLE.COM");
        using var other = factory.ClientFor("OTHER\\realmjane");
        Assert.Equal(HttpStatusCode.Created, (await owner.PostAsJsonAsync("/api/teams", new { @namespace = "realm-team", displayName = "Realm Team" }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.OK, (await owner.PostAsJsonAsync("/api/teams/realm-team/members", new { account = "TEST\\realmjane", owner = false }, Json, ct)).StatusCode);

        var me = await kerberos.GetFromJsonAsync<JsonElement>("/api/me", Json, ct);
        Assert.Contains("realm-team", me.GetProperty("teams").EnumerateArray().Select(team => team.GetProperty("namespace").GetString()));
        Assert.Equal(1, me.GetProperty("unreadNotifications").GetInt32());
        Assert.Equal(HttpStatusCode.OK, (await kerberos.GetAsync("/api/teams/realm-team", ct)).StatusCode);

        // Another domain with the same username is someone else.
        var stranger = await other.GetFromJsonAsync<JsonElement>("/api/me", Json, ct);
        Assert.Empty(stranger.GetProperty("teams").EnumerateArray());
        Assert.Equal(0, stranger.GetProperty("unreadNotifications").GetInt32());
    }

    [Fact]
    public async Task A_withdraw_or_restore_that_changes_what_runs_needs_approval_again()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\requeuer");
        using var admin = factory.ClientFor("TEST\\admin");
        using var viewer = factory.ClientFor("TEST\\requeueviewer");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillAndMcpPackage("requeuer", "db", "Node server."), "1.0.0"))
        {
            await PublishLiveAsync(owner, "requeuer", "db", form);
        }

        Assert.Equal(HttpStatusCode.NoContent, (await admin.PostAsJsonAsync("/api/admin/reviews/requeuer/db", new { decision = "approve" }, Json, ct)).StatusCode);
        await AssertVisible(viewer, "requeuer", "db");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillAndMcpPackage("requeuer", "db", "Python server.", command: "python"), "1.1.0"))
        {
            await PublishLiveAsync(owner, "requeuer", "db", form);
        }

        await AssertHidden(viewer, "requeuer", "db");

        // Withdrawn, the approved node version is live; the admin approves it again.
        Assert.Equal(HttpStatusCode.NoContent, (await owner.PutAsync("/api/packages/requeuer/db/versions/1.1.0/yank", null, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await admin.PostAsJsonAsync("/api/admin/reviews/requeuer/db", new { decision = "approve" }, Json, ct)).StatusCode);
        await AssertVisible(viewer, "requeuer", "db");

        // Restoring the python version makes something unreviewed live again.
        Assert.Equal(HttpStatusCode.NoContent, (await owner.DeleteAsync("/api/packages/requeuer/db/versions/1.1.0/yank", ct)).StatusCode);
        await AssertHidden(viewer, "requeuer", "db");

        // The same after a purge: the version below was declined.
        Assert.Equal(HttpStatusCode.NoContent, (await admin.PostAsJsonAsync("/api/admin/reviews/requeuer/db", new { decision = "decline", note = "Python is not allowed." }, Json, ct)).StatusCode);
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillAndMcpPackage("requeuer", "db", "Back to node."), "1.2.0"))
        {
            await PublishLiveAsync(owner, "requeuer", "db", form);
        }

        Assert.Equal(HttpStatusCode.NoContent, (await admin.PostAsJsonAsync("/api/admin/reviews/requeuer/db", new { decision = "approve" }, Json, ct)).StatusCode);
        await AssertVisible(viewer, "requeuer", "db");
        Assert.Equal(HttpStatusCode.NoContent, (await admin.DeleteAsync("/api/admin/packages/requeuer/db/versions/1.2.0", ct)).StatusCode);
        await AssertHidden(viewer, "requeuer", "db");
    }

    [Fact]
    public async Task An_mcp_document_that_names_a_key_twice_is_refused()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\dupkeys");
        var manifest = """{ "version": 2, "source": { "id": "dupkeys", "name": "dupkeys", "description": "Test." }, "packages": [ { "id": "db", "name": "db", "description": "Twice.", "components": [ { "kind": "mcpServer", "id": "db", "path": "mcp/db.json" } ] } ] }""";
        foreach (var document in new[]
        {
            """{ "$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json", "mcpServers": { "db": { "type": "stdio", "command": "node" }, "db": { "type": "stdio", "command": "python" } } }""",
            """{ "$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json", "mcpServers": { "db": { "type": "stdio", "command": "node", "command": "python" } } }""",
        })
        {
            using var form = SamplePackages.PublishForm(SamplePackages.Zip(new Dictionary<string, string> { ["agent-plugins.json"] = manifest, ["mcp/db.json"] = document }), "1.0.0");
            var response = await owner.PostAsync("/api/packages/dupkeys/db/versions", form, ct);
            Assert.Equal(HttpStatusCode.UnprocessableEntity, response.StatusCode);
        }
    }

    [Fact]
    public async Task A_person_keeps_the_u_namespace_after_the_team_that_forced_it_is_deleted()
    {
        var ct = TestContext.Current.CancellationToken;
        using var founder = factory.ClientFor("TEST\\teamfounder");
        using var person = factory.ClientFor("TEST\\kimberly");
        Assert.Equal(HttpStatusCode.Created, (await founder.PostAsJsonAsync("/api/teams", new { @namespace = "kimberly", displayName = "Kimberly's namesake" }, Json, ct)).StatusCode);
        Assert.Equal("u-kimberly", (await person.GetFromJsonAsync<JsonElement>("/api/me", Json, ct)).GetProperty("namespace").GetString());
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("u-kimberly", "notes"), "1.0.0"))
        {
            await PublishLiveAsync(person, "u-kimberly", "notes", form);
        }

        Assert.Equal(HttpStatusCode.NoContent, (await founder.DeleteAsync("/api/teams/kimberly", ct)).StatusCode);

        Assert.Equal("u-kimberly", (await person.GetFromJsonAsync<JsonElement>("/api/me", Json, ct)).GetProperty("namespace").GetString());
        Assert.Equal(HttpStatusCode.NoContent, (await person.PutAsync("/api/packages/u-kimberly/notes/versions/1.0.0/yank", null, ct)).StatusCode);
    }

    [Fact]
    public async Task A_block_is_one_row_whatever_the_case_and_an_unblock_lifts_it()
    {
        var ct = TestContext.Current.CancellationToken;
        using var admin = factory.ClientFor("TEST\\admin");
        using var blocked = factory.ClientFor("TEST\\casey");
        Assert.Equal(HttpStatusCode.NoContent, (await admin.PutAsJsonAsync("/api/admin/blocks/TEST%5Ccasey", new { reason = "Lower." }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await admin.PutAsJsonAsync("/api/admin/blocks/TEST%5CCASEY", new { reason = "Upper." }, Json, ct)).StatusCode);
        Assert.Single((await admin.GetFromJsonAsync<JsonElement>("/api/admin/blocks", Json, ct)).EnumerateArray(), block => block.GetProperty("account").GetString()!.Equals("TEST\\casey", StringComparison.OrdinalIgnoreCase));

        Assert.Equal(HttpStatusCode.NoContent, (await admin.DeleteAsync("/api/admin/blocks/test%5Ccasey", ct)).StatusCode);
        Assert.Equal(HttpStatusCode.Created, (await blocked.PostAsJsonAsync("/api/teams", new { @namespace = "casey-team", displayName = "Casey" }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NotFound, (await admin.DeleteAsync("/api/admin/blocks/TEST%5Ccasey", ct)).StatusCode);
    }

    [Fact]
    public async Task A_decline_note_is_for_the_owners_and_a_hidden_revoke_for_nobody_else()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\decliner");
        using var admin = factory.ClientFor("TEST\\admin");
        using var listed = factory.ClientFor("TEST\\declinelisted");
        using var stranger = factory.ClientFor("TEST\\declinestranger");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillAndMcpPackage("decliner", "db", "Server."), "1.0.0"))
        {
            await PublishLiveAsync(owner, "decliner", "db", form);
        }

        Assert.Equal(HttpStatusCode.NoContent, (await admin.PostAsJsonAsync("/api/admin/reviews/decliner/db", new { decision = "decline", note = "Internal reason." }, Json, ct)).StatusCode);
        await Share(owner, "decliner/db", new { visibility = "private", users = new[] { "TEST\\declinelisted" } });
        Assert.Equal("Internal reason.", (await PublicReview(owner, "decliner/db")).GetProperty("note").GetString());
        Assert.Equal(JsonValueKind.Null, (await PublicReview(listed, "decliner/db")).GetProperty("note").ValueKind);

        // Public again and still waiting, then revoked: only its owners ever saw it, so only they hear of the revoke.
        await Share(owner, "decliner/db", new { visibility = "inherit" });
        Assert.Equal(HttpStatusCode.NoContent, (await owner.PutAsync("/api/packages/decliner/db/revoke", null, ct)).StatusCode);
        Assert.DoesNotContain("decliner/db", Revoked(await stranger.GetFromJsonAsync<JsonElement>("/api/index", Json, ct)));
        Assert.Contains("decliner/db", Revoked(await owner.GetFromJsonAsync<JsonElement>("/api/index", Json, ct)));
    }

    [Fact]
    public async Task A_suggestion_decided_and_withdrawn_at_once_has_one_outcome()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\racer");
        using var suggester = factory.ClientFor("TEST\\racesuggester");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("racer", "notes"), "1.0.0"))
        {
            await PublishLiveAsync(owner, "racer", "notes", form);
        }

        for (var round = 0; round < 5; round++)
        {
            var id = (await Suggest(suggester, "racer", "notes", SamplePackages.SkillPackage("racer", "notes", $"Round {round}."), "Race.")).GetProperty("id").GetInt64();
            var accept = owner.PostAsJsonAsync($"/api/suggestions/{id}", new { decision = "accept" }, Json, ct);
            var withdraw = suggester.DeleteAsync($"/api/suggestions/{id}", ct);
            var statuses = new[] { (await accept).StatusCode, (await withdraw).StatusCode };
            Assert.Single(statuses, status => status is HttpStatusCode.OK or HttpStatusCode.NoContent);
            Assert.Single(statuses, HttpStatusCode.Conflict);
        }
    }

    [Fact]
    public async Task Two_admins_blocking_one_account_at_once_both_succeed()
    {
        var ct = TestContext.Current.CancellationToken;
        using var first = factory.ClientFor("TEST\\admin");
        using var second = factory.ClientFor("TEST\\admin");
        for (var round = 0; round < 5; round++)
        {
            var statuses = await Task.WhenAll(
                first.PutAsJsonAsync($"/api/admin/blocks/TEST%5Cracy{round}", new { reason = "One." }, Json, ct),
                second.PutAsJsonAsync($"/api/admin/blocks/TEST%5Cracy{round}", new { reason = "Two." }, Json, ct));
            Assert.All(statuses, response => Assert.Equal(HttpStatusCode.NoContent, response.StatusCode));
        }
    }

    [Fact]
    public async Task A_server_too_long_for_the_approval_column_can_still_be_approved()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\longspec");
        using var stranger = factory.ClientFor("TEST\\longwatcher");
        using var admin = factory.ClientFor("TEST\\admin");
        static byte[] Package(string note)
        {
            var args = string.Join(", ", Enumerable.Range(0, 10).Select(index => $"\"--{index}={new string('x', 1000)}{note}\""));
            return SamplePackages.Zip(new Dictionary<string, string>
            {
                ["agent-plugins.json"] = """{ "version": 2, "source": { "id": "longspec", "name": "longspec", "description": "Test." }, "packages": [ { "id": "big", "name": "Big", "description": "A long launch spec.", "components": [ { "kind": "mcpServer", "id": "big", "path": "mcp/big.json" } ] } ] }""",
                ["mcp/big.json"] = $$"""{ "$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json", "mcpServers": { "big": { "type": "stdio", "command": "node", "args": [{{args}}] } } }""",
            });
        }

        using (var form = SamplePackages.PublishForm(Package("a"), "1.0.0"))
        {
            Assert.True((await PublishLiveAsync(owner, "longspec", "big", form)).GetProperty("waitingForPublicReview").GetBoolean());
        }

        Assert.Equal(HttpStatusCode.NoContent, (await admin.PostAsJsonAsync("/api/admin/reviews/longspec/big", new { decision = "approve" }, Json, ct)).StatusCode);
        await AssertVisible(stranger, "longspec", "big");
        using (var form = SamplePackages.PublishForm(Package("a"), "1.0.1"))
        {
            Assert.False((await PublishLiveAsync(owner, "longspec", "big", form)).GetProperty("waitingForPublicReview").GetBoolean());
        }

        using (var form = SamplePackages.PublishForm(Package("b"), "1.0.2"))
        {
            Assert.True((await PublishLiveAsync(owner, "longspec", "big", form)).GetProperty("waitingForPublicReview").GetBoolean());
        }
    }
}
