using System.IO.Compression;
using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Marketplace.Api.Auth;
using Marketplace.Api.Data;
using Microsoft.Extensions.DependencyInjection;
using Xunit;

namespace Marketplace.Api.Tests;

public sealed partial class MarketplaceApiTests(MarketplaceApiFactory factory) : IClassFixture<MarketplaceApiFactory>
{
    private static readonly JsonSerializerOptions Json = new(JsonSerializerDefaults.Web);

    [Fact]
    public async Task Health_is_anonymous_and_reports_dev_header()
    {
        using var client = factory.CreateClient();
        var health = await client.GetFromJsonAsync<JsonElement>("/api/health", Json, TestContext.Current.CancellationToken);
        Assert.Equal("0.1.0", health.GetProperty("minimumClientVersion").GetString());
        Assert.Contains("DevHeader", health.GetProperty("authSchemes").EnumerateArray().Select(scheme => scheme.GetString()));
        Assert.False(health.GetProperty("adGroups").GetBoolean());
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
        var published = await PublishLiveAsync(client, "pubone", "greet", form);
        Assert.False(published.GetProperty("waitingForPublicReview").GetBoolean());
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
        var downloaded = await download.Content.ReadAsByteArrayAsync(TestContext.Current.CancellationToken);
        await AssertHeadThenNotModified(client, "/api/sources/pubone/archive", etag, downloaded.Length);
        using var zip = new ZipArchive(new MemoryStream(downloaded));
        var manifestEntry = zip.GetEntry("agent-plugins.json");
        Assert.NotNull(manifestEntry);
        using var manifestStream = manifestEntry.Open();
        var manifest = await JsonDocument.ParseAsync(manifestStream, cancellationToken: TestContext.Current.CancellationToken);
        Assert.Equal("pubone", manifest.RootElement.GetProperty("source").GetProperty("id").GetString());
        var component = manifest.RootElement.GetProperty("packages")[0].GetProperty("components")[0];
        Assert.Equal("greet/skills/greet", component.GetProperty("path").GetString());
        Assert.NotNull(zip.GetEntry("greet/skills/greet/SKILL.md"));

        var index = await client.GetFromJsonAsync<JsonElement>("/api/index", Json, TestContext.Current.CancellationToken);
        var entry = index.GetProperty("packages").EnumerateArray().Single(candidate => candidate.GetProperty("id").GetString() == "pubone/greet");
        Assert.Equal("personal", entry.GetProperty("lane").GetString());
        Assert.Equal(["greeting", "demo"], entry.GetProperty("tags").EnumerateArray().Select(tag => tag.GetString()).ToArray());
        Assert.Equal("TEST\\pubone", entry.GetProperty("publisher").GetProperty("account").GetString());
    }

    [Fact]
    public async Task Cross_site_writes_are_refused()
    {
        var ct = TestContext.Current.CancellationToken;
        using var client = factory.ClientFor("TEST\\crosssite");
        using var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("crosssite", "greet"), "1.0.0");
        using var crossSite = new HttpRequestMessage(HttpMethod.Post, "/api/packages/crosssite/greet/versions") { Content = form };
        crossSite.Headers.Add("Sec-Fetch-Site", "cross-site");
        var refused = await client.SendAsync(crossSite, ct);
        Assert.Equal(HttpStatusCode.Forbidden, refused.StatusCode);
        Assert.Contains("another website", await Title(refused));
        Assert.DoesNotContain("crosssite/greet/1.0.0.zip", factory.Store.Paths);

        using var read = new HttpRequestMessage(HttpMethod.Get, "/api/me");
        read.Headers.Add("Sec-Fetch-Site", "cross-site");
        Assert.Equal(HttpStatusCode.OK, (await client.SendAsync(read, ct)).StatusCode);

        using var portalForm = SamplePackages.PublishForm(SamplePackages.SkillPackage("crosssite", "greet"), "1.0.0");
        using var portal = new HttpRequestMessage(HttpMethod.Post, "/api/packages/crosssite/greet/versions") { Content = portalForm };
        portal.Headers.Add("Sec-Fetch-Site", "same-origin");
        Assert.Equal(HttpStatusCode.Created, (await client.SendAsync(portal, ct)).StatusCode);
    }

    [Fact]
    public void Namespace_is_valid_when_no_ascii_survives()
    {
        foreach (var username in new[] { "иван", "___" })
        {
            Assert.Matches(IdentityResolver.SourceIdPattern(), IdentityResolver.NamespaceFor(username));
        }

        Assert.NotEqual(IdentityResolver.NamespaceFor("иван"), IdentityResolver.NamespaceFor("___"));
        Assert.Equal(IdentityResolver.NamespaceFor("иван"), IdentityResolver.NamespaceFor("ИВАН"));
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
        await PublishLiveAsync(curator, "official", "review", allowed);
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
        var ct = TestContext.Current.CancellationToken;
        var firstArchive = SamplePackages.SkillPackage("versioner", "tool", "Version one.");
        using var first = SamplePackages.PublishForm(firstArchive, "1.0.0");
        await PublishLiveAsync(client, "versioner", "tool", first);
        using var again = SamplePackages.PublishForm(SamplePackages.SkillPackage("versioner", "tool"), "1.0.0");
        var conflict = await client.PostAsync("/api/packages/versioner/tool/versions", again, ct);
        Assert.Equal(HttpStatusCode.Conflict, conflict.StatusCode);
        Assert.Contains("Publish 1.0.1 or later", await Title(conflict));

        // An older version goes live without review but must not take over the listing.
        using var older = SamplePackages.PublishForm(SamplePackages.SkillPackage("versioner", "tool", "Old news.", extraFile: "old.txt"), "0.9.0");
        await PublishLiveAsync(client, "versioner", "tool", older);
        Assert.Equal("Version one.", IndexEntry(await client.GetFromJsonAsync<JsonElement>("/api/index", Json, ct), "versioner/tool").GetProperty("description").GetString());
        using var newer = SamplePackages.PublishForm(SamplePackages.SkillPackage("versioner", "tool", "Version two.", extraFile: "new.txt"), "1.1.0");
        await PublishLiveAsync(client, "versioner", "tool", newer);

        var index = await client.GetFromJsonAsync<JsonElement>("/api/index", Json, TestContext.Current.CancellationToken);
        var entry = index.GetProperty("packages").EnumerateArray().Single(candidate => candidate.GetProperty("id").GetString() == "versioner/tool");
        Assert.Equal("1.1.0", entry.GetProperty("version").GetString());
        Assert.Equal("Version two.", entry.GetProperty("description").GetString());

        var download = await client.GetAsync("/api/packages/versioner/tool/versions/1.0.0/archive", ct);
        Assert.Equal(HttpStatusCode.OK, download.StatusCode);
        Assert.Equal(firstArchive, await download.Content.ReadAsByteArrayAsync(ct));
        Assert.Equal("versioner-tool-1.0.0.zip", download.Content.Headers.ContentDisposition?.FileName?.Trim('"'));
        using var head = new HttpRequestMessage(HttpMethod.Head, "/api/packages/versioner/tool/versions/1.0.0/archive");
        var headResponse = await client.SendAsync(head, ct);
        Assert.Equal(HttpStatusCode.OK, headResponse.StatusCode);
        Assert.Equal(download.Headers.ETag, headResponse.Headers.ETag);

        var archive = await client.GetByteArrayAsync("/api/sources/versioner/archive", TestContext.Current.CancellationToken);
        using var zip = new ZipArchive(new MemoryStream(archive));
        Assert.NotNull(zip.GetEntry("tool/skills/tool/new.txt"));
        Assert.Null(zip.GetEntry("tool/skills/tool/old.txt"));

        var yank = await client.PutAsync("/api/packages/versioner/tool/versions/1.1.0/yank", null, TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.NoContent, yank.StatusCode);
        var afterYank = await client.GetFromJsonAsync<JsonElement>("/api/index", Json, TestContext.Current.CancellationToken);
        var yanked = afterYank.GetProperty("packages").EnumerateArray().Single(candidate => candidate.GetProperty("id").GetString() == "versioner/tool");
        Assert.Equal("1.0.0", yanked.GetProperty("version").GetString());
        Assert.Equal("Version one.", yanked.GetProperty("description").GetString());
        var detail = await client.GetFromJsonAsync<JsonElement>("/api/packages/versioner/tool", Json, TestContext.Current.CancellationToken);
        Assert.True(detail.GetProperty("versions").EnumerateArray().Single(candidate => candidate.GetProperty("version").GetString() == "1.1.0").GetProperty("yanked").GetBoolean());

        Assert.Equal(HttpStatusCode.NoContent, (await client.DeleteAsync("/api/packages/versioner/tool/versions/1.1.0/yank", ct)).StatusCode);
        var restored = IndexEntry(await client.GetFromJsonAsync<JsonElement>("/api/index", Json, ct), "versioner/tool");
        Assert.Equal("1.1.0", restored.GetProperty("version").GetString());
        Assert.Equal("Version two.", restored.GetProperty("description").GetString());
        var missing = await client.PutAsync("/api/packages/versioner/tool/versions/9.9.9/yank", null, ct);
        Assert.Equal(HttpStatusCode.NotFound, missing.StatusCode);
        Assert.Equal("versioner/tool 9.9.9 was not found, or you do not have access to it.", await Title(missing));
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
        await PublishLiveAsync(publisher, "metrics", "counter", form);

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

    [Fact]
    public async Task Installed_base_counts_each_recent_heartbeat_once_per_package()
    {
        var ct = TestContext.Current.CancellationToken;
        using var publisher = factory.ClientFor("TEST\\basecount");
        foreach (var packageId in new[] { "one", "two" })
        {
            using var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("basecount", packageId), "1.0.0");
            await PublishLiveAsync(publisher, "basecount", packageId, form);
        }

        using (var scope = factory.Services.CreateScope())
        {
            var db = scope.ServiceProvider.GetRequiredService<MarketplaceDbContext>();
            var now = DateTime.UtcNow;
            foreach (var (account, occurredAt, installed) in new[]
            {
                ("TEST\\base-a", now.AddDays(-1), new[] { "basecount/one", "basecount/two" }),
                ("TEST\\base-b", now.AddHours(-1), new[] { "basecount/one" }),
                ("TEST\\base-c", now.AddDays(-40), new[] { "basecount/one", "basecount/two" }),
            })
            {
                db.Heartbeats.Add(new Heartbeat { Account = account, OccurredAt = occurredAt, ReceivedAt = occurredAt, ClientVersion = "0.1.0", OsBuild = "10.0.26100", Installed = installed, ChecksJson = "{}" });
            }

            await db.SaveChangesAsync(ct);
        }

        var index = await publisher.GetFromJsonAsync<JsonElement>("/api/index", Json, ct);
        Assert.Equal(2, IndexEntry(index, "basecount/one").GetProperty("installedBase").GetInt32());
        Assert.Equal(1, IndexEntry(index, "basecount/two").GetProperty("installedBase").GetInt32());
    }

    [Fact]
    public async Task Namespace_access_rule_hides_it_from_everyone_not_listed()
    {
        using var owner = factory.ClientFor("TEST\\gatekeeper");
        using var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("gatekeeper", "tool"), "1.0.0");
        await PublishLiveAsync(owner, "gatekeeper", "tool", form);
        var set = await owner.PutAsJsonAsync("/api/access/gatekeeper", new { users = new[] { "TEST\\friend", "  " }, groups = new[] { "Platform Team" } }, Json, TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.OK, set.StatusCode);
        var document = await set.Content.ReadFromJsonAsync<JsonElement>(Json, TestContext.Current.CancellationToken);
        Assert.Equal("gatekeeper", document.GetProperty("target").GetString());
        Assert.Equal(["TEST\\friend"], document.GetProperty("users").EnumerateArray().Select(user => user.GetProperty("account").GetString()).ToArray());
        Assert.Equal("private", document.GetProperty("visibility").GetString());

        using var stranger = factory.ClientFor("TEST\\stranger2");
        await AssertHidden(stranger, "gatekeeper", "tool");
        using var member = factory.ClientFor("TEST\\member", "platform team");
        await AssertVisible(member, "gatekeeper", "tool");
        using var friend = factory.ClientFor("TEST\\Friend");
        await AssertVisible(friend, "gatekeeper", "tool");
        using var friendUpn = factory.ClientFor("friend@test.example");
        await AssertVisible(friendUpn, "gatekeeper", "tool");
        using var admin = factory.ClientFor("TEST\\admin");
        await AssertVisible(admin, "gatekeeper", "tool");
        await AssertVisible(owner, "gatekeeper", "tool");

        var index = await owner.GetFromJsonAsync<JsonElement>("/api/index", Json, TestContext.Current.CancellationToken);
        var entry = index.GetProperty("packages").EnumerateArray().Single(candidate => candidate.GetProperty("id").GetString() == "gatekeeper/tool");
        Assert.True(entry.GetProperty("restricted").GetBoolean());
        var read = await member.GetAsync("/api/access/gatekeeper", TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.Forbidden, read.StatusCode);
        var mine = await owner.GetFromJsonAsync<JsonElement>("/api/access/gatekeeper", Json, TestContext.Current.CancellationToken);
        Assert.Equal(["Platform Team"], mine.GetProperty("groups").EnumerateArray().Select(group => group.GetString()).ToArray());

        Assert.Equal(HttpStatusCode.Forbidden, (await stranger.PutAsJsonAsync("/api/access/gatekeeper", new { users = new[] { "TEST\\stranger2" } }, Json, TestContext.Current.CancellationToken)).StatusCode);
        Assert.Equal(HttpStatusCode.NotFound, (await owner.PutAsJsonAsync("/api/access/gatekeeper/missing", new { users = new[] { "TEST\\friend" } }, Json, TestContext.Current.CancellationToken)).StatusCode);
        Assert.Equal(HttpStatusCode.UnprocessableEntity, (await owner.PutAsJsonAsync("/api/access/gatekeeper", new { users = new[] { new string('x', 300) } }, Json, TestContext.Current.CancellationToken)).StatusCode);

        var cleared = await owner.PutAsJsonAsync("/api/access/gatekeeper", new { users = Array.Empty<string>(), groups = Array.Empty<string>() }, Json, TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.OK, cleared.StatusCode);
        await AssertVisible(stranger, "gatekeeper", "tool");
    }

    [Fact]
    public async Task Package_rule_overrides_namespace_rule_and_filters_the_archive()
    {
        using var owner = factory.ClientFor("TEST\\partial");
        foreach (var packageId in new[] { "open", "secret" })
        {
            using var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("partial", packageId), "1.0.0");
            await PublishLiveAsync(owner, "partial", packageId, form);
        }

        Assert.Equal(HttpStatusCode.OK, (await owner.PutAsJsonAsync("/api/access/partial", new { groups = new[] { "Platform Team" } }, Json, TestContext.Current.CancellationToken)).StatusCode);
        Assert.Equal(HttpStatusCode.OK, (await owner.PutAsJsonAsync("/api/access/partial/secret", new { users = new[] { "carol" } }, Json, TestContext.Current.CancellationToken)).StatusCode);
        var full = await owner.GetAsync("/api/sources/partial/archive", TestContext.Current.CancellationToken);
        var fullEtag = full.Headers.ETag?.Tag;
        Assert.NotNull(fullEtag);

        using var member = factory.ClientFor("TEST\\teammate", "Platform Team");
        Assert.Equal(1, await PackageCount(member, "partial"));
        var memberIndex = await member.GetFromJsonAsync<JsonElement>("/api/index", Json, TestContext.Current.CancellationToken);
        Assert.Equal(["partial/open"], IndexIds(memberIndex, "partial/"));
        var first = await member.GetAsync("/api/sources/partial/archive", TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.OK, first.StatusCode);
        var etag = first.Headers.ETag?.Tag;
        Assert.NotNull(etag);
        Assert.NotEqual(fullEtag, etag);
        Assert.Equal(full.Content.Headers.LastModified, first.Content.Headers.LastModified);
        var bytes = await first.Content.ReadAsByteArrayAsync(TestContext.Current.CancellationToken);
        Assert.Equal(["open"], await ManifestPackageIds(bytes));
        using (var zip = new ZipArchive(new MemoryStream(bytes)))
        {
            Assert.NotNull(zip.GetEntry("open/skills/open/SKILL.md"));
            Assert.DoesNotContain(zip.Entries, entry => entry.FullName.StartsWith("secret/", StringComparison.Ordinal));
        }

        var second = await member.GetAsync("/api/sources/partial/archive", TestContext.Current.CancellationToken);
        Assert.Equal(etag, second.Headers.ETag?.Tag);
        await AssertHeadThenNotModified(member, "/api/sources/partial/archive", etag, length: null);

        using var carol = factory.ClientFor("TEST\\carol");
        Assert.Equal(1, await PackageCount(carol, "partial"));
        var carolIndex = await carol.GetFromJsonAsync<JsonElement>("/api/index", Json, TestContext.Current.CancellationToken);
        Assert.Equal(["partial/secret"], IndexIds(carolIndex, "partial/"));
        Assert.Equal(["secret"], await ManifestPackageIds(await carol.GetByteArrayAsync("/api/sources/partial/archive", TestContext.Current.CancellationToken)));

        Assert.Equal(HttpStatusCode.OK, (await owner.PutAsJsonAsync("/api/access/partial", new { }, Json, TestContext.Current.CancellationToken)).StatusCode);
        Assert.Equal(HttpStatusCode.OK, (await owner.PutAsJsonAsync("/api/access/partial/secret", new { }, Json, TestContext.Current.CancellationToken)).StatusCode);
        using var stranger = factory.ClientFor("TEST\\stranger3");
        Assert.Equal(2, await PackageCount(stranger, "partial"));
        Assert.Equal(fullEtag, (await stranger.GetAsync("/api/sources/partial/archive", TestContext.Current.CancellationToken)).Headers.ETag?.Tag);
    }

    [Fact]
    public async Task An_admins_mcp_server_needs_no_approval()
    {
        var ct = TestContext.Current.CancellationToken;
        using var admin = factory.ClientFor("TEST\\admin");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillAndMcpPackage("official", "adminpick", "Picked by an admin."), "1.0.0"))
        {
            Assert.False((await PublishLiveAsync(admin, "official", "adminpick", form)).GetProperty("waitingForPublicReview").GetBoolean());
        }

        using var stranger = factory.ClientFor("TEST\\adminpickfan");
        await AssertVisible(stranger, "official", "adminpick");
        Assert.Equal("approved", (await stranger.GetFromJsonAsync<JsonElement>("/api/packages/official/adminpick", Json, ct)).GetProperty("publicReview").GetProperty("state").GetString());
    }

    [Fact]
    public async Task Admin_resolves_reports()
    {
        var ct = TestContext.Current.CancellationToken;
        using var reporter = factory.ClientFor("TEST\\reporter");
        using var admin = factory.ClientFor("TEST\\admin");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("official", "flagged"), "1.0.0"))
        {
            await PublishLiveAsync(admin, "official", "flagged", form);
        }

        Assert.Equal(HttpStatusCode.NotFound, (await reporter.PostAsJsonAsync("/api/packages/someone/thing/reports", new { reason = "Leaks a token." }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.UnprocessableEntity, (await reporter.PostAsJsonAsync("/api/packages/official/flagged/reports", new { reason = " " }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.Accepted, (await reporter.PostAsJsonAsync("/api/packages/official/flagged/reports", new { reason = "Leaks a token." }, Json, ct)).StatusCode);
        var open = (await admin.GetFromJsonAsync<JsonElement>("/api/admin/summary", Json, ct)).GetProperty("openReports").GetInt32();
        var report = (await admin.GetFromJsonAsync<JsonElement>("/api/admin/reports", Json, ct)).EnumerateArray().First(candidate => candidate.GetProperty("reason").GetString() == "Leaks a token.");
        Assert.Equal("official/flagged", report.GetProperty("packageId").GetString());
        var id = report.GetProperty("id").GetInt64();
        Assert.Equal(HttpStatusCode.Forbidden, (await reporter.PostAsync($"/api/admin/reports/{id}/resolve", null, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await admin.PostAsync($"/api/admin/reports/{id}/resolve", null, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NotFound, (await admin.PostAsync("/api/admin/reports/999999/resolve", null, ct)).StatusCode);
        Assert.Equal(open - 1, (await admin.GetFromJsonAsync<JsonElement>("/api/admin/summary", Json, ct)).GetProperty("openReports").GetInt32());
        var resolved = (await admin.GetFromJsonAsync<JsonElement>("/api/admin/reports", Json, ct)).EnumerateArray().Single(candidate => candidate.GetProperty("id").GetInt64() == id);
        Assert.Equal("TEST\\admin", resolved.GetProperty("resolvedBy").GetString());
    }

    [Fact]
    public async Task Browser_uploads_are_wrapped_into_one_package()
    {
        Assert.SkipWhen(factory.ValidatorPath is null, "Wrapping uploads needs the Rust validator.");
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\uploader");

        using (var single = SamplePackages.UploadForm("1.0.0", new Dictionary<string, string> { ["SKILL.md"] = SamplePackages.Skill("uploader-notes", "Takes meeting notes.") }))
        {
            await PublishLiveAsync(owner, "uploader", "notes", single);
        }

        var files = await owner.GetFromJsonAsync<JsonElement>("/api/packages/uploader/notes/versions/1.0.0/files", Json, ct);
        Assert.Equal(["agent-plugins.json", "skills/notes/SKILL.md"], files.EnumerateArray().Select(file => file.GetProperty("path").GetString()).ToArray());
        Assert.Contains("name: notes", await owner.GetStringAsync("/api/packages/uploader/notes/versions/1.0.0/files/skills/notes/SKILL.md", ct));

        var folder = new Dictionary<string, string>
        {
            ["Tone Guide/SKILL.md"] = SamplePackages.Skill("tone", "Writes in our house tone."),
            ["Tone Guide/examples/good.md"] = "Warm and brief.\n",
        };
        using (var form = SamplePackages.UploadForm("1.0.0", folder))
        {
            await PublishLiveAsync(owner, "uploader", "tone", form);
        }

        Assert.Contains("skills/tone/examples/good.md", (await owner.GetFromJsonAsync<JsonElement>("/api/packages/uploader/tone/versions/1.0.0/files", Json, ct)).EnumerateArray().Select(file => file.GetProperty("path").GetString()));

        var pack = new Dictionary<string, string>
        {
            ["pack/tone/SKILL.md"] = SamplePackages.Skill("tone", "Writes in our house tone."),
            ["pack/summarize/SKILL.md"] = SamplePackages.Skill("summarize", "Summarizes meeting notes."),
        };
        using (var form = SamplePackages.UploadForm("1.0.0", pack, new Dictionary<string, string> { ["name"] = "Writing Pack", ["description"] = "Everything for writing." }))
        {
            await PublishLiveAsync(owner, "uploader", "writing", form);
        }

        var entry = (await owner.GetFromJsonAsync<JsonElement>("/api/index", Json, ct)).GetProperty("packages").EnumerateArray().Single(candidate => candidate.GetProperty("id").GetString() == "uploader/writing");
        Assert.Equal("Writing Pack", entry.GetProperty("name").GetString());
        Assert.Equal("Everything for writing.", entry.GetProperty("description").GetString());

        var zipped = SamplePackages.Zip(new Dictionary<string, string> { ["lint/SKILL.md"] = SamplePackages.Skill("lint", "Checks style.") });
        using (var form = SamplePackages.PublishForm(zipped, "1.0.0"))
        {
            await PublishLiveAsync(owner, "uploader", "lint", form);
        }

        var mcp = new Dictionary<string, string> { ["database.json"] = """{ "$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json", "mcpServers": { "database": { "type": "stdio", "command": "node", "args": ["server.js"] } } }""" };
        using (var form = SamplePackages.UploadForm("1.0.0", mcp))
        {
            await PublishLiveAsync(owner, "uploader", "database", form);
        }

        Assert.Contains("mcp/database.json", (await owner.GetFromJsonAsync<JsonElement>("/api/packages/uploader/database/versions/1.0.0/files", Json, ct)).EnumerateArray().Select(file => file.GetProperty("path").GetString()));
    }

    [Fact]
    public async Task Unsafe_uploads_are_refused()
    {
        Assert.SkipWhen(factory.ValidatorPath is null, "The secret scan runs in the Rust validator.");
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\careless");
        using (var escape = SamplePackages.UploadForm("1.0.0", new Dictionary<string, string> { ["../SKILL.md"] = SamplePackages.Skill("escape", "Escapes.") }))
        {
            Assert.Equal(HttpStatusCode.UnprocessableEntity, (await owner.PostAsync("/api/packages/careless/escape/versions", escape, ct)).StatusCode);
        }

        using (var intruder = factory.ClientFor("TEST\\intruder2"))
        using (var foreign = SamplePackages.UploadForm("1.0.0", new Dictionary<string, string> { ["SKILL.md"] = SamplePackages.Skill("x", "X.") }))
        {
            Assert.Equal(HttpStatusCode.Forbidden, (await intruder.PostAsync("/api/packages/careless/x/versions", foreign, ct)).StatusCode);
        }

        var files = new Dictionary<string, string>
        {
            ["agent-plugins.json"] = """{"version":2,"source":{"id":"careless","name":"c","description":"c"},"packages":[{"id":"leaky","components":[{"kind":"skill","path":"skills/leaky"}]}]}""",
            ["skills/leaky/SKILL.md"] = SamplePackages.Skill("leaky", "Leaks."),
            ["skills/leaky/.env"] = "TOKEN=1\n",
        };
        using var form = SamplePackages.PublishForm(SamplePackages.Zip(files), "1.0.0");
        var response = await owner.PostAsync("/api/packages/careless/leaky/versions", form, ct);
        Assert.Equal(HttpStatusCode.UnprocessableEntity, response.StatusCode);
        var problem = await response.Content.ReadFromJsonAsync<JsonElement>(Json, ct);
        Assert.Contains(problem.GetProperty("errors").EnumerateArray(), error => error.GetProperty("path").GetString() == "skills/leaky/.env");
    }

    [Fact]
    public async Task Portal_and_downloads_are_served_beside_the_api()
    {
        var ct = TestContext.Current.CancellationToken;
        using var browser = factory.CreateClient();
        var page = await browser.GetAsync("/p/someone/thing", ct);
        Assert.Equal(HttpStatusCode.OK, page.StatusCode);
        Assert.Equal("text/html", page.Content.Headers.ContentType?.MediaType);
        Assert.Contains("no-cache", page.Headers.CacheControl?.ToString());
        Assert.Contains("script-src 'self'", page.Headers.GetValues("Content-Security-Policy").Single());
        Assert.Equal("nosniff", page.Headers.GetValues("X-Content-Type-Options").Single());

        var missingApi = await browser.GetAsync("/api/nope", ct);
        Assert.Equal(HttpStatusCode.NotFound, missingApi.StatusCode);
        Assert.Equal("application/problem+json", missingApi.Content.Headers.ContentType?.MediaType);
        Assert.Equal(HttpStatusCode.NotFound, (await browser.GetAsync("/missing.js", ct)).StatusCode);

        var manifest = await browser.GetAsync("/downloads/manifest.json", ct);
        Assert.True(manifest.Headers.CacheControl?.NoStore);
        var installer = await browser.GetAsync("/downloads/releases/0.1.0/Agent-Plugins.AppImage", ct);
        Assert.Equal(HttpStatusCode.OK, installer.StatusCode);
        Assert.Equal("attachment", installer.Content.Headers.ContentDisposition?.DispositionType);
        Assert.Contains("immutable", installer.Headers.CacheControl?.ToString());
    }

    [Fact]
    public async Task Personal_namespaces_are_claimed_and_never_shared()
    {
        var ct = TestContext.Current.CancellationToken;
        using var first = factory.ClientFor("TEST\\christopher.johnson");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("christopher-john", "notes"), "1.0.0"))
        {
            await PublishLiveAsync(first, "christopher-john", "notes", form);
        }

        // Truncation gives the second account the same derived name; it gets the next free one instead.
        using var second = factory.ClientFor("TEST\\christopher.johnston");
        var me = await second.GetFromJsonAsync<JsonElement>("/api/me", Json, ct);
        Assert.Equal("christopher-jo-2", me.GetProperty("namespace").GetString());
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("christopher-john", "notes"), "1.0.1"))
        {
            Assert.Equal(HttpStatusCode.Forbidden, (await second.PostAsync("/api/packages/christopher-john/notes/versions", form, ct)).StatusCode);
        }

        Assert.Equal(HttpStatusCode.Forbidden, (await second.PutAsync("/api/packages/christopher-john/notes/versions/1.0.0/yank", null, ct)).StatusCode);
        Assert.Equal("christopher-john", (await first.GetFromJsonAsync<JsonElement>("/api/me", Json, ct)).GetProperty("namespace").GetString());

        // Until someone publishes, every account deriving a name owns it, so none may set its access list.
        using var fresh = factory.ClientFor("TEST\\fresh");
        Assert.Equal(HttpStatusCode.Conflict, (await fresh.PutAsJsonAsync("/api/access/fresh", new { users = new[] { "TEST\\fresh" } }, Json, ct)).StatusCode);

        // Reserved names never become personal namespaces.
        using var official = factory.ClientFor("TEST\\official");
        var officialMe = await official.GetFromJsonAsync<JsonElement>("/api/me", Json, ct);
        Assert.Equal("u-official", officialMe.GetProperty("namespace").GetString());
        Assert.DoesNotContain("official", officialMe.GetProperty("namespaces").EnumerateArray().Select(ns => ns.GetString()));
        using var founder = factory.ClientFor("TEST\\platform.founder");
        Assert.Equal(HttpStatusCode.Created, (await founder.PostAsJsonAsync("/api/teams", new { @namespace = "team-platform", displayName = "Platform Team" }, Json, ct)).StatusCode);
        using var team = factory.ClientFor("TEST\\team.platform");
        Assert.Equal("u-team-platform", (await team.GetFromJsonAsync<JsonElement>("/api/me", Json, ct)).GetProperty("namespace").GetString());

        // An admin may not create someone's personal namespace, which would claim it for the admin.
        using var admin = factory.ClientFor("TEST\\admin");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("unclaimed", "tool"), "1.0.0"))
        {
            Assert.Equal(HttpStatusCode.Forbidden, (await admin.PostAsync("/api/packages/unclaimed/tool/versions", form, ct)).StatusCode);
        }
    }

    [Fact]
    public async Task A_retried_publish_adopts_its_own_stored_archive()
    {
        var ct = TestContext.Current.CancellationToken;
        using var client = factory.ClientFor("TEST\\retrier");
        var archive = SamplePackages.SkillPackage("retrier", "tool");
        await factory.Store.PutAsync("retrier/tool/1.0.0.zip", archive, ct);
        using (var form = SamplePackages.PublishForm(archive, "1.0.0"))
        {
            await PublishLiveAsync(client, "retrier", "tool", form);
        }

        await factory.Store.PutAsync("retrier/tool/1.1.0.zip", SamplePackages.SkillPackage("retrier", "tool", "Something else."), ct);
        using var different = SamplePackages.PublishForm(archive, "1.1.0");
        var response = await client.PostAsync("/api/packages/retrier/tool/versions", different, ct);
        Assert.Equal(HttpStatusCode.Conflict, response.StatusCode);
        Assert.Contains("different content", await Title(response));
    }

    [Fact]
    public async Task Publish_input_is_checked_not_silently_changed()
    {
        var ct = TestContext.Current.CancellationToken;
        using var client = factory.ClientFor("TEST\\strict");
        var archive = SamplePackages.SkillPackage("strict", "tool");
        async Task<string> Rejected(string version, string? tags = null, string? changelog = null)
        {
            using var form = SamplePackages.PublishForm(archive, version, tags, changelog);
            var response = await client.PostAsync("/api/packages/strict/tool/versions", form, ct);
            Assert.Equal(HttpStatusCode.UnprocessableEntity, response.StatusCode);
            return await Title(response);
        }

        Assert.Contains("pre-release", await Rejected("1.0.0-beta.1"));
        Assert.Contains("not a version number", await Rejected("1.0"));
        Assert.Contains("\"Bad Tag!\"".ToLowerInvariant(), await Rejected("1.0.0", "ok, Bad Tag!"));
        Assert.Contains("at most 10 tags", await Rejected("1.0.0", string.Join(',', Enumerable.Range(1, 11).Select(n => $"t{n}"))));
        Assert.Contains("changelog", await Rejected("1.0.0", changelog: new string('x', 4097)));
    }

    [Fact]
    public async Task Malformed_input_is_a_client_error_with_a_reason()
    {
        var ct = TestContext.Current.CancellationToken;
        using var client = factory.ClientFor("TEST\\sloppy");
        using var badJson = new StringContent("""{"events":[{"kind":"install","occurredAt":"nope"}]}""", System.Text.Encoding.UTF8, "application/json");
        var response = await client.PostAsync("/api/events", badJson, ct);
        Assert.Equal(HttpStatusCode.BadRequest, response.StatusCode);
        var problem = await response.Content.ReadFromJsonAsync<JsonElement>(Json, ct);
        Assert.Contains("occurredAt", problem.GetProperty("detail").GetString());

        var heartbeat = new { kind = "heartbeat", occurredAt = DateTimeOffset.UtcNow, clientVersion = "0.1.0", agents = new string?[] { null, "cursor" }, installed = new string?[] { null } };
        var accepted = await client.PostAsJsonAsync("/api/events", new { events = new object?[] { null, heartbeat } }, Json, ct);
        Assert.Equal(HttpStatusCode.Accepted, accepted.StatusCode);
        var counts = await accepted.Content.ReadFromJsonAsync<JsonElement>(Json, ct);
        Assert.Equal(1, counts.GetProperty("accepted").GetInt32());
        Assert.Equal(1, counts.GetProperty("rejected").GetInt32());

        async Task<string> Rejected(byte[] archive)
        {
            using var form = SamplePackages.PublishForm(archive, "1.0.0");
            var rejected = await client.PostAsync("/api/packages/sloppy/tool/versions", form, ct);
            Assert.Equal(HttpStatusCode.UnprocessableEntity, rejected.StatusCode);
            return await Title(rejected);
        }

        Assert.Contains("source.id", await Rejected(SamplePackages.Zip(new Dictionary<string, string> { ["agent-plugins.json"] = """{"version":2,"source":[],"packages":[]}""" })));
        Assert.Contains("version 2", await Rejected(SamplePackages.Zip(new Dictionary<string, string> { ["agent-plugins.json"] = """{"version":2.5}""" })));
        Assert.Contains("package has no id", await Rejected(SamplePackages.Zip(new Dictionary<string, string> { ["agent-plugins.json"] = """{"version":2,"source":{"id":"sloppy"},"packages":[{"id":7}]}""" })));
        Assert.Contains("more than once", await Rejected(DuplicateEntryZip()));
    }

    [Fact]
    public async Task Large_files_download_instead_of_failing()
    {
        var ct = TestContext.Current.CancellationToken;
        using var client = factory.ClientFor("TEST\\bigfile");
        var archive = SamplePackages.Zip(new Dictionary<string, string>
        {
            ["agent-plugins.json"] = """{"version":2,"source":{"id":"bigfile","name":"b","description":"b"},"packages":[{"id":"tool","name":"Tool","description":"Has a big file.","components":[{"kind":"skill","path":"skills/tool"}]}]}""",
            ["skills/tool/SKILL.md"] = SamplePackages.Skill("tool", "Has a big file."),
            ["skills/tool/data.txt"] = new string('a', MarketplaceEndpointsMaxFileView + 1),
        });
        using (var form = SamplePackages.PublishForm(archive, "1.0.0"))
        {
            await PublishLiveAsync(client, "bigfile", "tool", form);
        }

        var response = await client.GetAsync("/api/packages/bigfile/tool/versions/1.0.0/files/skills/tool/data.txt", ct);
        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
        Assert.Equal("application/octet-stream", response.Content.Headers.ContentType?.MediaType);
        Assert.Equal(MarketplaceEndpointsMaxFileView + 1, (await response.Content.ReadAsByteArrayAsync(ct)).Length);
    }

    private const int MarketplaceEndpointsMaxFileView = Endpoints.MarketplaceEndpoints.MaxFileView;

    private static byte[] DuplicateEntryZip()
    {
        using var output = new MemoryStream();
        using (var zip = new ZipArchive(output, ZipArchiveMode.Create, leaveOpen: true))
        {
            foreach (var name in new[] { "agent-plugins.json", "skills/a/SKILL.md", "skills/a/skill.md" })
            {
                using var stream = zip.CreateEntry(name).Open();
                stream.Write("{}"u8);
            }
        }

        return output.ToArray();
    }

    private static async Task<string> Title(HttpResponseMessage response) =>
        (await response.Content.ReadFromJsonAsync<JsonElement>(Json, TestContext.Current.CancellationToken)).GetProperty("title").GetString()!;

    private static JsonElement IndexEntry(JsonElement index, string id) =>
        index.GetProperty("packages").EnumerateArray().Single(candidate => candidate.GetProperty("id").GetString() == id);

    /// <summary>Publishes; every version goes live at once.</summary>
    private static async Task<JsonElement> PublishLiveAsync(HttpClient client, string ns, string packageId, MultipartFormDataContent form)
    {
        var response = await client.PostAsync($"/api/packages/{ns}/{packageId}/versions", form, TestContext.Current.CancellationToken);
        Assert.True(response.StatusCode == HttpStatusCode.Created, await response.Content.ReadAsStringAsync(TestContext.Current.CancellationToken));
        return await response.Content.ReadFromJsonAsync<JsonElement>(Json, TestContext.Current.CancellationToken);
    }

    /// <summary>HEAD answers with the ETag the GET served, and a GET carrying the HEAD's ETag answers 304 with it.</summary>
    private static async Task AssertHeadThenNotModified(HttpClient client, string url, string etag, long? length)
    {
        using var headRequest = new HttpRequestMessage(HttpMethod.Head, url);
        var head = await client.SendAsync(headRequest, TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.OK, head.StatusCode);
        Assert.Equal(etag, head.Headers.ETag?.Tag);
        if (length is not null)
        {
            Assert.Equal(length, head.Content.Headers.ContentLength);
        }

        using var conditional = new HttpRequestMessage(HttpMethod.Get, url);
        conditional.Headers.IfNoneMatch.ParseAdd(head.Headers.ETag!.Tag);
        var notModified = await client.SendAsync(conditional, TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.NotModified, notModified.StatusCode);
        Assert.Equal(etag, notModified.Headers.ETag?.Tag);
    }

    private static async Task AssertHidden(HttpClient client, string ns, string packageId)
    {
        var catalog = await client.GetFromJsonAsync<JsonElement>("/api/catalog", Json, TestContext.Current.CancellationToken);
        Assert.DoesNotContain(catalog.GetProperty("sources").EnumerateArray(), source => source.GetProperty("sourceId").GetString() == ns);
        var index = await client.GetFromJsonAsync<JsonElement>("/api/index", Json, TestContext.Current.CancellationToken);
        Assert.DoesNotContain(index.GetProperty("packages").EnumerateArray(), package => package.GetProperty("id").GetString() == $"{ns}/{packageId}");
        Assert.Equal(HttpStatusCode.NotFound, (await client.GetAsync($"/api/sources/{ns}/archive", TestContext.Current.CancellationToken)).StatusCode);
        Assert.Equal(HttpStatusCode.NotFound, (await client.GetAsync($"/api/packages/{ns}/{packageId}", TestContext.Current.CancellationToken)).StatusCode);
        Assert.Equal(HttpStatusCode.NotFound, (await client.GetAsync($"/api/stats/packages/{ns}/{packageId}", TestContext.Current.CancellationToken)).StatusCode);
    }

    private static async Task AssertVisible(HttpClient client, string ns, string packageId)
    {
        var catalog = await client.GetFromJsonAsync<JsonElement>("/api/catalog", Json, TestContext.Current.CancellationToken);
        Assert.Contains(catalog.GetProperty("sources").EnumerateArray(), source => source.GetProperty("sourceId").GetString() == ns);
        var index = await client.GetFromJsonAsync<JsonElement>("/api/index", Json, TestContext.Current.CancellationToken);
        Assert.Contains(index.GetProperty("packages").EnumerateArray(), package => package.GetProperty("id").GetString() == $"{ns}/{packageId}");
        Assert.Equal(HttpStatusCode.OK, (await client.GetAsync($"/api/sources/{ns}/archive", TestContext.Current.CancellationToken)).StatusCode);
        Assert.Equal(HttpStatusCode.OK, (await client.GetAsync($"/api/packages/{ns}/{packageId}", TestContext.Current.CancellationToken)).StatusCode);
    }

    private static async Task<int> PackageCount(HttpClient client, string ns)
    {
        var catalog = await client.GetFromJsonAsync<JsonElement>("/api/catalog", Json, TestContext.Current.CancellationToken);
        return catalog.GetProperty("sources").EnumerateArray().Single(source => source.GetProperty("sourceId").GetString() == ns).GetProperty("packageCount").GetInt32();
    }

    private static string[] IndexIds(JsonElement index, string prefix) =>
        index.GetProperty("packages").EnumerateArray()
            .Select(package => package.GetProperty("id").GetString()!)
            .Where(id => id.StartsWith(prefix, StringComparison.Ordinal))
            .ToArray();

    private static async Task<string[]> ManifestPackageIds(byte[] archive)
    {
        using var zip = new ZipArchive(new MemoryStream(archive));
        using var stream = zip.GetEntry("agent-plugins.json")!.Open();
        var manifest = await JsonDocument.ParseAsync(stream, cancellationToken: TestContext.Current.CancellationToken);
        return manifest.RootElement.GetProperty("packages").EnumerateArray().Select(package => package.GetProperty("id").GetString()!).ToArray();
    }
}
