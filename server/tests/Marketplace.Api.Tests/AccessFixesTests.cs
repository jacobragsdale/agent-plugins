using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Xunit;

namespace Marketplace.Api.Tests;

/// <summary>Access defects found by the live end-to-end audit of c821355.</summary>
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
}
