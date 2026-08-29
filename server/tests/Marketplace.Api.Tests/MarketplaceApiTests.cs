using System.IO.Compression;
using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Xunit;

namespace Marketplace.Api.Tests;

public sealed class MarketplaceApiTests(MarketplaceApiFactory factory) : IClassFixture<MarketplaceApiFactory>
{
    private static readonly JsonSerializerOptions Json = new(JsonSerializerDefaults.Web);

    [Fact]
    public async Task Health_is_anonymous_and_reports_dev_header()
    {
        using var client = factory.CreateClient();
        var health = await client.GetFromJsonAsync<JsonElement>("/api/health", Json, TestContext.Current.CancellationToken);
        Assert.Equal("0.1.0", health.GetProperty("minimumClientVersion").GetString());
        Assert.Contains("DevHeader", health.GetProperty("authSchemes").EnumerateArray().Select(scheme => scheme.GetString()));
    }

    [Fact]
    public async Task Anonymous_request_is_challenged()
    {
        using var client = factory.CreateClient();
        var response = await client.GetAsync("/api/me", TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.Unauthorized, response.StatusCode);
    }

    [Fact]
    public async Task Me_derives_namespace_from_windows_account()
    {
        using var client = factory.ClientFor("TEST\\Jane.Doe");
        var me = await client.GetFromJsonAsync<JsonElement>("/api/me", Json, TestContext.Current.CancellationToken);
        Assert.Equal("TEST\\Jane.Doe", me.GetProperty("account").GetString());
        Assert.Equal("jane-doe", me.GetProperty("namespace").GetString());
        Assert.False(me.GetProperty("admin").GetBoolean());
    }

    [Fact]
    public async Task Publish_lists_the_namespace_in_the_catalog_and_serves_its_archive()
    {
        using var client = factory.ClientFor("TEST\\pubone");
        var archive = SamplePackages.SkillPackage("pubone", "greet");
        using var form = SamplePackages.PublishForm(archive, "1.0.0", "greeting, demo", "First release.");
        var response = await client.PostAsync("/api/packages/pubone/greet/versions", form, TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.Created, response.StatusCode);
        var published = await response.Content.ReadFromJsonAsync<JsonElement>(Json, TestContext.Current.CancellationToken);
        Assert.Equal("pubone/greet", published.GetProperty("id").GetString());
        Assert.Equal("1.0.0", published.GetProperty("version").GetString());
        Assert.Contains("pubone/greet/1.0.0.zip", factory.Store.Paths);

        var catalog = await client.GetFromJsonAsync<JsonElement>("/api/catalog", Json, TestContext.Current.CancellationToken);
        Assert.Equal(1, catalog.GetProperty("version").GetInt32());
        var source = catalog.GetProperty("sources").EnumerateArray().Single(candidate => candidate.GetProperty("sourceId").GetString() == "pubone");
        Assert.Equal("https://marketplace.test/api/sources/pubone/archive", source.GetProperty("url").GetString());
        Assert.Equal(1, source.GetProperty("packageCount").GetInt32());

        var download = await client.GetAsync("/api/sources/pubone/archive", TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.OK, download.StatusCode);
        var etag = download.Headers.ETag?.Tag;
        Assert.NotNull(etag);
        using var zip = new ZipArchive(new MemoryStream(await download.Content.ReadAsByteArrayAsync(TestContext.Current.CancellationToken)));
        var manifestEntry = zip.GetEntry("agent-plugins.json");
        Assert.NotNull(manifestEntry);
        using var manifestStream = manifestEntry.Open();
        var manifest = await JsonDocument.ParseAsync(manifestStream, cancellationToken: TestContext.Current.CancellationToken);
        Assert.Equal("pubone", manifest.RootElement.GetProperty("source").GetProperty("id").GetString());
        var component = manifest.RootElement.GetProperty("packages")[0].GetProperty("components")[0];
        Assert.Equal("greet/skills/greet", component.GetProperty("path").GetString());
        Assert.NotNull(zip.GetEntry("greet/skills/greet/SKILL.md"));

        using var conditional = new HttpRequestMessage(HttpMethod.Get, "/api/sources/pubone/archive");
        conditional.Headers.IfNoneMatch.ParseAdd(etag);
        var notModified = await client.SendAsync(conditional, TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.NotModified, notModified.StatusCode);

        var index = await client.GetFromJsonAsync<JsonElement>("/api/index", Json, TestContext.Current.CancellationToken);
        var entry = index.GetProperty("packages").EnumerateArray().Single(candidate => candidate.GetProperty("id").GetString() == "pubone/greet");
        Assert.Equal("personal", entry.GetProperty("lane").GetString());
        Assert.Equal(["greeting", "demo"], entry.GetProperty("tags").EnumerateArray().Select(tag => tag.GetString()).ToArray());
        Assert.Equal("TEST\\pubone", entry.GetProperty("publisher").GetProperty("account").GetString());
    }

    [Fact]
    public async Task Publishing_into_another_namespace_is_forbidden()
    {
        using var client = factory.ClientFor("TEST\\intruder");
        using var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("victim", "greet"), "1.0.0");
        var response = await client.PostAsync("/api/packages/victim/greet/versions", form, TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.Forbidden, response.StatusCode);
    }

    [Fact]
    public async Task Official_namespace_requires_allowlisting()
    {
        using var stranger = factory.ClientFor("TEST\\stranger");
        using var denied = SamplePackages.PublishForm(SamplePackages.SkillPackage("official", "review"), "1.0.0");
        var refused = await stranger.PostAsync("/api/packages/official/review/versions", denied, TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.Forbidden, refused.StatusCode);

        using var curator = factory.ClientFor("TEST\\curator");
        using var allowed = SamplePackages.PublishForm(SamplePackages.SkillPackage("official", "review"), "1.0.0");
        var accepted = await curator.PostAsync("/api/packages/official/review/versions", allowed, TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.Created, accepted.StatusCode);
        var index = await curator.GetFromJsonAsync<JsonElement>("/api/index", Json, TestContext.Current.CancellationToken);
        var entry = index.GetProperty("packages").EnumerateArray().Single(candidate => candidate.GetProperty("id").GetString() == "official/review");
        Assert.Equal("official", entry.GetProperty("lane").GetString());
    }

    [Fact]
    public async Task Manifest_namespace_must_match_the_route()
    {
        using var client = factory.ClientFor("TEST\\mismatch");
        using var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("someoneelse", "greet"), "1.0.0");
        var response = await client.PostAsync("/api/packages/mismatch/greet/versions", form, TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.UnprocessableEntity, response.StatusCode);
        var problem = await response.Content.ReadFromJsonAsync<JsonElement>(Json, TestContext.Current.CancellationToken);
        Assert.Contains("source.id", problem.GetProperty("title").GetString());
    }

    [Fact]
    public async Task Duplicate_version_conflicts_and_newer_version_wins()
    {
        using var client = factory.ClientFor("TEST\\versioner");
        using var first = SamplePackages.PublishForm(SamplePackages.SkillPackage("versioner", "tool"), "1.0.0");
        Assert.Equal(HttpStatusCode.Created, (await client.PostAsync("/api/packages/versioner/tool/versions", first, TestContext.Current.CancellationToken)).StatusCode);
        using var again = SamplePackages.PublishForm(SamplePackages.SkillPackage("versioner", "tool"), "1.0.0");
        Assert.Equal(HttpStatusCode.Conflict, (await client.PostAsync("/api/packages/versioner/tool/versions", again, TestContext.Current.CancellationToken)).StatusCode);
        using var older = SamplePackages.PublishForm(SamplePackages.SkillPackage("versioner", "tool", extraFile: "old.txt"), "0.9.0");
        Assert.Equal(HttpStatusCode.Created, (await client.PostAsync("/api/packages/versioner/tool/versions", older, TestContext.Current.CancellationToken)).StatusCode);
        using var newer = SamplePackages.PublishForm(SamplePackages.SkillPackage("versioner", "tool", extraFile: "new.txt"), "1.1.0");
        Assert.Equal(HttpStatusCode.Created, (await client.PostAsync("/api/packages/versioner/tool/versions", newer, TestContext.Current.CancellationToken)).StatusCode);

        var index = await client.GetFromJsonAsync<JsonElement>("/api/index", Json, TestContext.Current.CancellationToken);
        var entry = index.GetProperty("packages").EnumerateArray().Single(candidate => candidate.GetProperty("id").GetString() == "versioner/tool");
        Assert.Equal("1.1.0", entry.GetProperty("version").GetString());

        var archive = await client.GetByteArrayAsync("/api/sources/versioner/archive", TestContext.Current.CancellationToken);
        using var zip = new ZipArchive(new MemoryStream(archive));
        Assert.NotNull(zip.GetEntry("tool/skills/tool/new.txt"));
        Assert.Null(zip.GetEntry("tool/skills/tool/old.txt"));

        var yank = await client.PostAsync("/api/packages/versioner/tool/versions/1.1.0/yank", null, TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.NoContent, yank.StatusCode);
        var afterYank = await client.GetFromJsonAsync<JsonElement>("/api/index", Json, TestContext.Current.CancellationToken);
        var yanked = afterYank.GetProperty("packages").EnumerateArray().Single(candidate => candidate.GetProperty("id").GetString() == "versioner/tool");
        Assert.Equal("1.0.0", yanked.GetProperty("version").GetString());
        var detail = await client.GetFromJsonAsync<JsonElement>("/api/packages/versioner/tool", Json, TestContext.Current.CancellationToken);
        Assert.True(detail.GetProperty("versions").EnumerateArray().Single(candidate => candidate.GetProperty("version").GetString() == "1.1.0").GetProperty("yanked").GetBoolean());
    }

    [Fact]
    public async Task Broken_archive_is_rejected_with_validation_detail()
    {
        using var client = factory.ClientFor("TEST\\broken");
        var archive = SamplePackages.Zip(new Dictionary<string, string>
        {
            ["agent-plugins.json"] = """{"version":2,"source":{"id":"broken","name":"b","description":"b"},"packages":[{"id":"nope","components":[{"kind":"skill","path":"skills/nope"}]}]}""",
        });
        using var form = SamplePackages.PublishForm(archive, "1.0.0");
        var response = await client.PostAsync("/api/packages/broken/nope/versions", form, TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.UnprocessableEntity, response.StatusCode);
        var problem = await response.Content.ReadFromJsonAsync<JsonElement>(Json, TestContext.Current.CancellationToken);
        Assert.NotEqual(JsonValueKind.Undefined, problem.ValueKind);
        if (factory.ValidatorPath is not null)
        {
            Assert.True(problem.TryGetProperty("errors", out var errors) && errors.GetArrayLength() > 0, "the Rust validator should report the missing skill directory");
        }
    }

    [Fact]
    public async Task Events_feed_installs_and_installed_base()
    {
        using var publisher = factory.ClientFor("TEST\\metrics");
        using var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("metrics", "counter"), "1.0.0");
        Assert.Equal(HttpStatusCode.Created, (await publisher.PostAsync("/api/packages/metrics/counter/versions", form, TestContext.Current.CancellationToken)).StatusCode);

        var now = DateTimeOffset.UtcNow;
        foreach (var account in new[] { "TEST\\userone", "TEST\\usertwo" })
        {
            using var user = factory.ClientFor(account);
            var batch = new
            {
                events = new object[]
                {
                    new { kind = "install", occurredAt = now.AddMinutes(-5), clientVersion = "0.1.0", packageId = "metrics/counter", version = "1.0.0", agents = new[] { "claude-code", "cursor" } },
                    new { kind = "install", occurredAt = now.AddMinutes(-5), clientVersion = "0.1.0", packageId = "metrics/counter", version = "1.0.0", agents = new[] { "claude-code", "cursor" } },
                    new { kind = "heartbeat", occurredAt = now.AddMinutes(-1), clientVersion = "0.1.0", osBuild = "10.0.26100", agents = new[] { "claude-code" }, installed = new[] { "metrics/counter" }, checks = new Dictionary<string, string> { ["auth.identity"] = "ok", ["host.clock"] = "fail" } },
                },
            };
            var response = await user.PostAsJsonAsync("/api/events", batch, Json, TestContext.Current.CancellationToken);
            Assert.Equal(HttpStatusCode.Accepted, response.StatusCode);
            var accepted = await response.Content.ReadFromJsonAsync<JsonElement>(Json, TestContext.Current.CancellationToken);
            Assert.Equal(2, accepted.GetProperty("accepted").GetInt32());
            Assert.Equal(1, accepted.GetProperty("duplicates").GetInt32());
        }

        var stats = await publisher.GetFromJsonAsync<JsonElement>("/api/stats/packages/metrics/counter", Json, TestContext.Current.CancellationToken);
        Assert.Equal(2, stats.GetProperty("installs").GetInt32());
        Assert.Equal(2, stats.GetProperty("installedBase").GetInt32());
        Assert.Equal(2, stats.GetProperty("agentMix").GetProperty("cursor").GetInt32());

        var index = await publisher.GetFromJsonAsync<JsonElement>("/api/index", Json, TestContext.Current.CancellationToken);
        var entry = index.GetProperty("packages").EnumerateArray().Single(candidate => candidate.GetProperty("id").GetString() == "metrics/counter");
        Assert.Equal(2, entry.GetProperty("installs").GetInt32());
        Assert.Equal(2, entry.GetProperty("installedBase").GetInt32());

        using var stranger = factory.ClientFor("TEST\\nobody");
        Assert.Equal(HttpStatusCode.Forbidden, (await stranger.GetAsync("/api/admin/summary", TestContext.Current.CancellationToken)).StatusCode);
        using var admin = factory.ClientFor("TEST\\admin");
        var summary = await admin.GetFromJsonAsync<JsonElement>("/api/admin/summary", Json, TestContext.Current.CancellationToken);
        Assert.True(summary.GetProperty("activeUsers").GetProperty("day").GetInt32() >= 2);
        Assert.Equal(2, summary.GetProperty("preflightFailures").GetProperty("host.clock").GetInt32());
    }
}
