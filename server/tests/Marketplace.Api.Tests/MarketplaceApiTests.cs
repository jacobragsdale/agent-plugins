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
        var published = await PublishLiveAsync(client, "pubone", "greet", form);
        Assert.Equal("pending", published.GetProperty("reviewState").GetString());
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
        using var first = SamplePackages.PublishForm(SamplePackages.SkillPackage("versioner", "tool"), "1.0.0");
        await PublishLiveAsync(client, "versioner", "tool", first);
        using var again = SamplePackages.PublishForm(SamplePackages.SkillPackage("versioner", "tool"), "1.0.0");
        Assert.Equal(HttpStatusCode.Conflict, (await client.PostAsync("/api/packages/versioner/tool/versions", again, TestContext.Current.CancellationToken)).StatusCode);
        using var older = SamplePackages.PublishForm(SamplePackages.SkillPackage("versioner", "tool", extraFile: "old.txt"), "0.9.0");
        await PublishLiveAsync(client, "versioner", "tool", older);
        using var newer = SamplePackages.PublishForm(SamplePackages.SkillPackage("versioner", "tool", extraFile: "new.txt"), "1.1.0");
        await PublishLiveAsync(client, "versioner", "tool", newer);

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
    public async Task Namespace_access_rule_hides_it_from_everyone_not_listed()
    {
        using var owner = factory.ClientFor("TEST\\gatekeeper");
        using var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("gatekeeper", "tool"), "1.0.0");
        await PublishLiveAsync(owner, "gatekeeper", "tool", form);
        var set = await owner.PutAsJsonAsync("/api/access/gatekeeper", new { users = new[] { "TEST\\friend", "  " }, groups = new[] { "Platform Team" } }, Json, TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.OK, set.StatusCode);
        var document = await set.Content.ReadFromJsonAsync<JsonElement>(Json, TestContext.Current.CancellationToken);
        Assert.Equal("gatekeeper", document.GetProperty("target").GetString());
        Assert.Equal(["TEST\\friend"], document.GetProperty("users").EnumerateArray().Select(user => user.GetString()).ToArray());

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
        using var headRequest = new HttpRequestMessage(HttpMethod.Head, "/api/sources/partial/archive");
        var head = await member.SendAsync(headRequest, TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.OK, head.StatusCode);
        Assert.Equal(etag, head.Headers.ETag?.Tag);
        Assert.Equal(bytes.Length, head.Content.Headers.ContentLength);
        using var conditional = new HttpRequestMessage(HttpMethod.Get, "/api/sources/partial/archive");
        conditional.Headers.IfNoneMatch.ParseAdd(etag);
        Assert.Equal(HttpStatusCode.NotModified, (await member.SendAsync(conditional, TestContext.Current.CancellationToken)).StatusCode);

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
    public async Task Team_namespace_belongs_to_the_group()
    {
        using var outsider = factory.ClientFor("TEST\\outsider");
        using var denied = SamplePackages.PublishForm(SamplePackages.SkillPackage("team-platform", "deploy"), "1.0.0");
        Assert.Equal(HttpStatusCode.Forbidden, (await outsider.PostAsync("/api/packages/team-platform/deploy/versions", denied, TestContext.Current.CancellationToken)).StatusCode);

        using var member = factory.ClientFor("TEST\\platformer", "platform team");
        var me = await member.GetFromJsonAsync<JsonElement>("/api/me", Json, TestContext.Current.CancellationToken);
        Assert.Contains("team-platform", me.GetProperty("namespaces").EnumerateArray().Select(ns => ns.GetString()));
        Assert.Equal(["platform team"], me.GetProperty("groups").EnumerateArray().Select(group => group.GetString()).ToArray());
        using var allowed = SamplePackages.PublishForm(SamplePackages.SkillPackage("team-platform", "deploy"), "1.0.0");
        await PublishLiveAsync(member, "team-platform", "deploy", allowed);

        var catalog = await outsider.GetFromJsonAsync<JsonElement>("/api/catalog", Json, TestContext.Current.CancellationToken);
        var source = catalog.GetProperty("sources").EnumerateArray().Single(candidate => candidate.GetProperty("sourceId").GetString() == "team-platform");
        Assert.Equal("Platform Team", source.GetProperty("publisher").GetString());
        var index = await outsider.GetFromJsonAsync<JsonElement>("/api/index", Json, TestContext.Current.CancellationToken);
        var entry = index.GetProperty("packages").EnumerateArray().Single(candidate => candidate.GetProperty("id").GetString() == "team-platform/deploy");
        Assert.Equal("team", entry.GetProperty("lane").GetString());
        Assert.Equal("Platform Team", entry.GetProperty("publisher").GetProperty("displayName").GetString());
        Assert.False(entry.GetProperty("restricted").GetBoolean());

        using var second = factory.ClientFor("TEST\\platformer2", "Platform Team");
        Assert.Equal(HttpStatusCode.OK, (await second.PutAsJsonAsync("/api/access/team-platform", new { groups = new[] { "Platform Team" } }, Json, TestContext.Current.CancellationToken)).StatusCode);
        await AssertHidden(outsider, "team-platform", "deploy");
        await AssertVisible(member, "team-platform", "deploy");
    }

    [Fact]
    public async Task First_version_waits_for_review_and_only_the_owner_sees_it()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\reviewee");
        using var stranger = factory.ClientFor("TEST\\onlooker");
        using var admin = factory.ClientFor("TEST\\admin");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("reviewee", "draft"), "1.0.0"))
        {
            var response = await owner.PostAsync("/api/packages/reviewee/draft/versions", form, ct);
            Assert.Equal(HttpStatusCode.Created, response.StatusCode);
            Assert.Equal("pending", (await response.Content.ReadFromJsonAsync<JsonElement>(Json, ct)).GetProperty("reviewState").GetString());
        }

        await AssertHidden(stranger, "reviewee", "draft");
        Assert.Equal(HttpStatusCode.NotFound, (await stranger.GetAsync("/api/packages/reviewee/draft/versions/1.0.0/files", ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NotFound, (await owner.GetAsync("/api/sources/reviewee/archive", ct)).StatusCode);
        Assert.DoesNotContain("reviewee/draft", IndexIds(await owner.GetFromJsonAsync<JsonElement>("/api/index", Json, ct), "reviewee/"));

        var detail = await owner.GetFromJsonAsync<JsonElement>("/api/packages/reviewee/draft", Json, ct);
        Assert.Equal(JsonValueKind.Null, detail.GetProperty("liveVersion").ValueKind);
        Assert.Equal("pending", detail.GetProperty("versions")[0].GetProperty("reviewState").GetString());
        var mine = await owner.GetFromJsonAsync<JsonElement>("/api/mine", Json, ct);
        Assert.Contains(mine.GetProperty("packages").EnumerateArray(), package => package.GetProperty("id").GetString() == "reviewee/draft");
        Assert.Equal("personal", mine.GetProperty("spaces")[0].GetProperty("lane").GetString());
        var files = await owner.GetFromJsonAsync<JsonElement>("/api/packages/reviewee/draft/versions/1.0.0/files", Json, ct);
        Assert.Equal(["agent-plugins.json", "skills/draft/SKILL.md"], files.EnumerateArray().Select(file => file.GetProperty("path").GetString()).ToArray());

        var queue = await admin.GetFromJsonAsync<JsonElement>("/api/admin/reviews", Json, ct);
        var pending = queue.EnumerateArray().Single(review => review.GetProperty("id").GetString() == "reviewee/draft");
        Assert.True(pending.GetProperty("firstVersion").GetBoolean());
        Assert.Equal(HttpStatusCode.Forbidden, (await owner.GetAsync("/api/admin/reviews", ct)).StatusCode);
        Assert.Equal(HttpStatusCode.UnprocessableEntity, (await admin.PostAsJsonAsync("/api/admin/reviews/reviewee/draft/1.0.0", new { decision = "reject" }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await admin.PostAsJsonAsync("/api/admin/reviews/reviewee/draft/1.0.0", new { decision = "reject", note = "Describe when to use it." }, Json, ct)).StatusCode);
        Assert.Equal(HttpStatusCode.Conflict, (await admin.PostAsJsonAsync("/api/admin/reviews/reviewee/draft/1.0.0", new { decision = "approve" }, Json, ct)).StatusCode);
        var rejected = (await owner.GetFromJsonAsync<JsonElement>("/api/packages/reviewee/draft", Json, ct)).GetProperty("versions")[0];
        Assert.Equal("rejected", rejected.GetProperty("reviewState").GetString());
        Assert.Equal("Describe when to use it.", rejected.GetProperty("reviewNote").GetString());

        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("reviewee", "draft"), "1.0.1"))
        {
            Assert.Equal("pending", (await PublishLiveAsync(owner, "reviewee", "draft", form)).GetProperty("reviewState").GetString());
        }

        await AssertVisible(stranger, "reviewee", "draft");
        var strangerDetail = await stranger.GetFromJsonAsync<JsonElement>("/api/packages/reviewee/draft", Json, ct);
        Assert.Equal(["1.0.1"], strangerDetail.GetProperty("versions").EnumerateArray().Select(version => version.GetProperty("version").GetString()).ToArray());
        Assert.Equal(HttpStatusCode.NotFound, (await stranger.GetAsync("/api/packages/reviewee/draft/versions/1.0.0/files", ct)).StatusCode);
        var skill = await stranger.GetAsync("/api/packages/reviewee/draft/versions/1.0.1/files/skills/draft/SKILL.md", ct);
        Assert.Equal("text/plain; charset=utf-8", skill.Content.Headers.ContentType?.ToString());
        Assert.Contains("name: draft", await skill.Content.ReadAsStringAsync(ct));
        Assert.Equal(HttpStatusCode.NotFound, (await stranger.GetAsync("/api/packages/reviewee/draft/versions/1.0.1/files/../secret", ct)).StatusCode);

        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("reviewee", "draft", "Greets people warmly."), "1.1.0"))
        {
            Assert.Equal("approved", (await PublishLiveAsync(owner, "reviewee", "draft", form)).GetProperty("reviewState").GetString());
        }

        var entry = (await stranger.GetFromJsonAsync<JsonElement>("/api/index", Json, ct)).GetProperty("packages").EnumerateArray().Single(candidate => candidate.GetProperty("id").GetString() == "reviewee/draft");
        Assert.Equal("1.1.0", entry.GetProperty("version").GetString());
        Assert.Equal("Greets people warmly.", entry.GetProperty("description").GetString());
    }

    [Fact]
    public async Task Versions_with_an_mcp_server_always_wait_and_leave_the_listing_alone()
    {
        var ct = TestContext.Current.CancellationToken;
        using var owner = factory.ClientFor("TEST\\mcpowner");
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("mcpowner", "db"), "1.0.0"))
        {
            await PublishLiveAsync(owner, "mcpowner", "db", form);
        }

        var etag = (await owner.GetAsync("/api/sources/mcpowner/archive", ct)).Headers.ETag?.Tag;
        using (var form = SamplePackages.PublishForm(SamplePackages.SkillAndMcpPackage("mcpowner", "db", "Queries the database."), "1.1.0", "database"))
        {
            var response = await owner.PostAsync("/api/packages/mcpowner/db/versions", form, ct);
            Assert.Equal(HttpStatusCode.Created, response.StatusCode);
            Assert.Equal("pending", (await response.Content.ReadFromJsonAsync<JsonElement>(Json, ct)).GetProperty("reviewState").GetString());
        }

        var before = (await owner.GetFromJsonAsync<JsonElement>("/api/index", Json, ct)).GetProperty("packages").EnumerateArray().Single(candidate => candidate.GetProperty("id").GetString() == "mcpowner/db");
        Assert.Equal("1.0.0", before.GetProperty("version").GetString());
        Assert.NotEqual("Queries the database.", before.GetProperty("description").GetString());
        Assert.Empty(before.GetProperty("tags").EnumerateArray());
        Assert.Equal(etag, (await owner.GetAsync("/api/sources/mcpowner/archive", ct)).Headers.ETag?.Tag);
        using var admin = factory.ClientFor("TEST\\admin");
        var pending = (await admin.GetFromJsonAsync<JsonElement>("/api/admin/reviews", Json, ct)).EnumerateArray().Single(review => review.GetProperty("id").GetString() == "mcpowner/db");
        Assert.False(pending.GetProperty("firstVersion").GetBoolean());
        Assert.Equal("1.0.0", pending.GetProperty("liveVersion").GetString());

        Assert.Equal(HttpStatusCode.NoContent, (await admin.PostAsJsonAsync("/api/admin/reviews/mcpowner/db/1.1.0", new { decision = "approve" }, Json, ct)).StatusCode);
        var after = (await owner.GetFromJsonAsync<JsonElement>("/api/index", Json, ct)).GetProperty("packages").EnumerateArray().Single(candidate => candidate.GetProperty("id").GetString() == "mcpowner/db");
        Assert.Equal("1.1.0", after.GetProperty("version").GetString());
        Assert.Equal("Queries the database.", after.GetProperty("description").GetString());
        Assert.Equal(["database"], after.GetProperty("tags").EnumerateArray().Select(tag => tag.GetString()).ToArray());
        Assert.NotEqual(etag, (await owner.GetAsync("/api/sources/mcpowner/archive", ct)).Headers.ETag?.Tag);
    }

    [Fact]
    public async Task Admin_publishes_go_live_without_review()
    {
        using var admin = factory.ClientFor("TEST\\admin");
        using var form = SamplePackages.PublishForm(SamplePackages.SkillPackage("official", "adminpick"), "1.0.0");
        var response = await admin.PostAsync("/api/packages/official/adminpick/versions", form, TestContext.Current.CancellationToken);
        Assert.Equal(HttpStatusCode.Created, response.StatusCode);
        Assert.Equal("approved", (await response.Content.ReadFromJsonAsync<JsonElement>(Json, TestContext.Current.CancellationToken)).GetProperty("reviewState").GetString());
    }

    [Fact]
    public async Task Admin_resolves_reports()
    {
        var ct = TestContext.Current.CancellationToken;
        using var reporter = factory.ClientFor("TEST\\reporter");
        Assert.Equal(HttpStatusCode.Accepted, (await reporter.PostAsJsonAsync("/api/reports", new { packageId = "someone/thing", reason = "Leaks a token." }, Json, ct)).StatusCode);
        using var admin = factory.ClientFor("TEST\\admin");
        var open = (await admin.GetFromJsonAsync<JsonElement>("/api/admin/summary", Json, ct)).GetProperty("openReports").GetInt32();
        var report = (await admin.GetFromJsonAsync<JsonElement>("/api/admin/reports", Json, ct)).EnumerateArray().First(candidate => candidate.GetProperty("reason").GetString() == "Leaks a token.");
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

        var edit = new Dictionary<string, string> { ["skills/tone/SKILL.md"] = SamplePackages.Skill("tone", "Writes in our house tone.", "Be warmer.") };
        using (var form = SamplePackages.UploadForm("1.0.1", edit, new Dictionary<string, string> { ["base"] = "1.0.0" }))
        {
            Assert.Equal("approved", (await PublishLiveAsync(owner, "uploader", "tone", form)).GetProperty("reviewState").GetString());
        }

        Assert.Contains("Be warmer.", await owner.GetStringAsync("/api/packages/uploader/tone/versions/1.0.1/files/skills/tone/SKILL.md", ct));
        Assert.Equal("Warm and brief.\n", await owner.GetStringAsync("/api/packages/uploader/tone/versions/1.0.1/files/skills/tone/examples/good.md", ct));

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

    /// <summary>Publishes and, when the version waits for review, approves it as an admin.</summary>
    private async Task<JsonElement> PublishLiveAsync(HttpClient client, string ns, string packageId, MultipartFormDataContent form)
    {
        var response = await client.PostAsync($"/api/packages/{ns}/{packageId}/versions", form, TestContext.Current.CancellationToken);
        Assert.True(response.StatusCode == HttpStatusCode.Created, await response.Content.ReadAsStringAsync(TestContext.Current.CancellationToken));
        var published = await response.Content.ReadFromJsonAsync<JsonElement>(Json, TestContext.Current.CancellationToken);
        if (published.GetProperty("reviewState").GetString() == "pending")
        {
            using var admin = factory.ClientFor("TEST\\admin");
            var review = await admin.PostAsJsonAsync($"/api/admin/reviews/{ns}/{packageId}/{published.GetProperty("version").GetString()}", new { decision = "approve" }, Json, TestContext.Current.CancellationToken);
            Assert.Equal(HttpStatusCode.NoContent, review.StatusCode);
        }

        return published;
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
