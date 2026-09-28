using System.Net;
using System.Net.Http.Headers;
using System.Net.Http.Json;
using System.Text.Json;
using Xunit;

namespace Marketplace.Api.Tests;

/// <summary>CI pipelines sign in with machine tokens and publish a whole repository to a team they are a member of.</summary>
public sealed partial class MarketplaceApiTests
{
    private static Dictionary<string, object> Repository(string name) => new() { ["repository"] = name };

    private static string Skill(string name, string body = "Follow the checklist.") =>
        $"---\nname: {name}\ndescription: Use it when {name} comes up.\n---\n\n# {name}\n\n{body}\n";

    private static MultipartFormDataContent RepositoryForm(IReadOnlyDictionary<string, string> files, bool dryRun = false, string? changelog = null)
    {
        var form = new MultipartFormDataContent();
        var archive = new ByteArrayContent(SamplePackages.Zip(files));
        archive.Headers.ContentType = new MediaTypeHeaderValue("application/zip");
        form.Add(archive, "archive", "repository.zip");
        if (dryRun)
        {
            form.Add(new StringContent("true"), "dryRun");
        }

        if (changelog is not null)
        {
            form.Add(new StringContent(changelog), "changelog");
        }

        return form;
    }

    private static string[] Statuses(JsonElement report) =>
        [.. report.GetProperty("packages").EnumerateArray().Select(package => $"{package.GetProperty("packageId").GetString()} {package.GetProperty("status").GetString()} {package.GetProperty("version").GetString()}")];

    [Fact]
    public async Task A_pipeline_publishes_what_changed_in_its_repository_to_its_team()
    {
        Assert.SkipWhen(factory.ValidatorPath is null, "Finding and wrapping a repository's skills needs the Rust validator.");
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\ci-owner");
        using var pipeline = factory.ClientForMachine(Repository("acme/skills"));
        Assert.Equal(HttpStatusCode.Created, (await owner.PostAsJsonAsync("/api/teams", new { @namespace = "ci-team", displayName = "CI Team" }, Json, ct)).StatusCode);
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("ci-team", "by-hand"), "1.0.0"))
        {
            await PublishLiveAsync(owner, "ci-team", "by-hand", form);
        }

        var files = new Dictionary<string, string>
        {
            ["README.md"] = "# Our skills\n",
            ["skills/review/SKILL.md"] = Skill("review"),
            ["skills/review/checklist.md"] = "- Tests pass\n",
            [".claude/skills/notes/SKILL.md"] = Skill("ci-team-notes"),
        };

        // Until an owner adds it, the pipeline learns exactly who it is and what to ask for, with the Accept header the CI templates send.
        using (var form = RepositoryForm(files, dryRun: true))
        {
            using var request = new HttpRequestMessage(HttpMethod.Post, "/api/namespaces/ci-team/publish") { Content = form };
            request.Headers.Accept.ParseAdd("text/markdown, application/problem+json");
            var refused = await pipeline.SendAsync(request, ct);
            Assert.Equal(HttpStatusCode.Forbidden, refused.StatusCode);
            Assert.Contains("github:acme/skills is not a member of ci-team. An owner of ci-team adds github:acme/skills under Members", await refused.Content.ReadAsStringAsync(ct));
        }

        var found = await owner.GetFromJsonAsync<JsonElement>("/api/directory?q=acme", Json, ct);
        Assert.Contains("github:acme/skills", found.GetProperty("people").EnumerateArray().Select(person => person.GetProperty("account").GetString()));
        Assert.Equal(HttpStatusCode.OK, (await owner.PostAsJsonAsync("/api/teams/ci-team/members", new { account = "github:acme/skills", owner = false }, Json, ct)).StatusCode);

        var me = await pipeline.GetFromJsonAsync<JsonElement>("/api/me", Json, ct);
        Assert.Equal("github:acme/skills", me.GetProperty("account").GetString());
        Assert.Equal(["ci-team"], me.GetProperty("namespaces").EnumerateArray().Select(ns => ns.GetString()));

        // A dry run shows the plan and stores nothing.
        using (var form = RepositoryForm(files, dryRun: true))
        {
            var planned = await pipeline.PostAsync("/api/namespaces/ci-team/publish", form, ct);
            Assert.Equal(HttpStatusCode.OK, planned.StatusCode);
            var report = await planned.Content.ReadFromJsonAsync<JsonElement>(Json, ct);
            Assert.True(report.GetProperty("dryRun").GetBoolean());
            Assert.Equal(["notes new 1.0.0", "review new 1.0.0"], Statuses(report));
            Assert.Equal(["by-hand"], report.GetProperty("elsewhere").EnumerateArray().Select(id => id.GetString()));
            Assert.Equal(".claude/skills/notes", report.GetProperty("packages")[0].GetProperty("path").GetString());
        }

        Assert.Equal(HttpStatusCode.NotFound, (await owner.GetAsync("/api/packages/ci-team/review", ct)).StatusCode);

        using (var form = RepositoryForm(files, changelog: "First release."))
        {
            var published = await pipeline.PostAsync("/api/namespaces/ci-team/publish", form, ct);
            Assert.Equal(HttpStatusCode.OK, published.StatusCode);
            var report = await published.Content.ReadFromJsonAsync<JsonElement>(Json, ct);
            Assert.All(report.GetProperty("packages").EnumerateArray(), package => Assert.True(package.GetProperty("published").GetBoolean()));
        }

        var review = await owner.GetFromJsonAsync<JsonElement>("/api/packages/ci-team/review", Json, ct);
        Assert.Equal("1.0.0", review.GetProperty("liveVersion").GetString());
        var latest = review.GetProperty("versions").EnumerateArray().Single();
        Assert.Equal("github:acme/skills", latest.GetProperty("publishedBy").GetString());
        Assert.Equal("First release.", latest.GetProperty("changelog").GetString());

        // A rerun changes nothing; an edited skill goes out as its next patch, and the report reads as Markdown.
        using (var form = RepositoryForm(files))
        {
            var report = await (await pipeline.PostAsync("/api/namespaces/ci-team/publish", form, ct)).Content.ReadFromJsonAsync<JsonElement>(Json, ct);
            Assert.Equal(["notes unchanged 1.0.0", "review unchanged 1.0.0"], Statuses(report));
        }

        files["skills/review/SKILL.md"] = Skill("review", "Follow the longer checklist.");
        using (var form = RepositoryForm(files))
        {
            using var request = new HttpRequestMessage(HttpMethod.Post, "/api/namespaces/ci-team/publish") { Content = form };
            request.Headers.Accept.ParseAdd("text/markdown");
            var response = await pipeline.SendAsync(request, ct);
            Assert.Equal(HttpStatusCode.OK, response.StatusCode);
            Assert.Equal("text/markdown", response.Content.Headers.ContentType?.MediaType);
            var markdown = await response.Content.ReadAsStringAsync(ct);
            Assert.Contains("### ci-team: published 1, 1 unchanged", markdown);
            Assert.Contains("| [review](https://marketplace.test/p/ci-team/review) | 1.0.0 → 1.0.1 | changed `skills/review/SKILL.md` |", markdown);
            Assert.Contains("not from this repository (left as they are): by-hand.", markdown);
        }

        // One broken skill holds back the whole repository.
        files["skills/review/SKILL.md"] = Skill("review", "A third version.");
        files["skills/broken/SKILL.md"] = "# No header\n";
        using (var form = RepositoryForm(files))
        {
            var refused = await pipeline.PostAsync("/api/namespaces/ci-team/publish", form, ct);
            Assert.Equal(HttpStatusCode.UnprocessableEntity, refused.StatusCode);
            var report = await refused.Content.ReadFromJsonAsync<JsonElement>(Json, ct);
            var broken = report.GetProperty("packages").EnumerateArray().Single(package => package.GetProperty("status").GetString() == "failed");
            Assert.Equal("skills/broken", broken.GetProperty("path").GetString());
            Assert.Contains("must start with a header", broken.GetProperty("errors")[0].GetProperty("message").GetString());
            Assert.All(report.GetProperty("packages").EnumerateArray(), package => Assert.False(package.GetProperty("published").GetBoolean()));
        }

        review = await owner.GetFromJsonAsync<JsonElement>("/api/packages/ci-team/review", Json, ct);
        Assert.Equal("1.0.1", review.GetProperty("liveVersion").GetString());

        // A pipeline publishes and does nothing else.
        Assert.Equal(HttpStatusCode.Forbidden, (await pipeline.PutAsync("/api/packages/ci-team/review/revoke", null, ct)).StatusCode);
        var created = await pipeline.PostAsJsonAsync("/api/teams", new { @namespace = "ci-own", displayName = "Mine" }, Json, ct);
        Assert.Equal(HttpStatusCode.Forbidden, created.StatusCode);
        Assert.Contains("A CI pipeline can only publish.", await created.Content.ReadAsStringAsync(ct));
    }

    [Fact]
    public async Task Machine_tokens_are_checked_and_never_make_an_admin()
    {
        var ct = TestContext.Current.CancellationToken;
        async Task<string> RefusedAsync(HttpClient client)
        {
            var response = await client.GetAsync("/api/me", ct);
            Assert.Equal(HttpStatusCode.Unauthorized, response.StatusCode);
            return (await response.Content.ReadFromJsonAsync<JsonElement>(Json, ct)).GetProperty("detail").GetString()!;
        }

        using (var client = factory.ClientForMachine(Repository("acme/skills"), audience: "https://elsewhere.test"))
        {
            Assert.Contains("was minted for https://elsewhere.test, not https://marketplace.test", await RefusedAsync(client));
        }

        using (var client = factory.ClientForMachine(Repository("acme/skills"), issuer: "https://stranger.test"))
        {
            Assert.Contains("comes from https://stranger.test, which this marketplace does not trust", await RefusedAsync(client));
        }

        using (var client = factory.ClientForMachine(Repository("acme/skills"), expires: DateTime.UtcNow.AddHours(-1)))
        {
            Assert.Contains("has expired", await RefusedAsync(client));
        }

        using (var client = factory.ClientForMachine(new Dictionary<string, object> { ["oid"] = "person", ["scp"] = "access" }, issuer: MarketplaceApiFactory.EntraIssuer, audience: "api://marketplace"))
        {
            Assert.Contains("belongs to a person", await RefusedAsync(client));
        }

        using (var client = factory.ClientForMachine(new Dictionary<string, object> { ["sub"] = "no repository" }))
        {
            Assert.Contains("has no repository claim", await RefusedAsync(client));
        }

        using (var entra = factory.ClientForMachine(new Dictionary<string, object> { ["oid"] = "3f2a0000-0000-0000-0000-000000000001", ["idtyp"] = "app" }, issuer: MarketplaceApiFactory.EntraIssuer, audience: "api://marketplace"))
        {
            var me = await entra.GetFromJsonAsync<JsonElement>("/api/me", Json, ct);
            Assert.Equal("app:3f2a0000-0000-0000-0000-000000000001", me.GetProperty("account").GetString());
            Assert.Equal("", me.GetProperty("namespace").GetString());
        }

        using var named = factory.ClientForMachine(Repository("acme/admin"));
        var admin = await named.GetFromJsonAsync<JsonElement>("/api/me", Json, ct);
        Assert.False(admin.GetProperty("admin").GetBoolean());
        Assert.Empty(admin.GetProperty("namespaces").EnumerateArray());
        Assert.Equal(HttpStatusCode.Forbidden, (await named.GetAsync("/api/admin/summary", ct)).StatusCode);
    }
}
