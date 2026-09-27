using System.Net;
using Marketplace.Api.Configuration;
using Marketplace.Api.Storage;
using Microsoft.Extensions.Logging.Abstractions;
using Microsoft.Extensions.Options;
using Xunit;

namespace Marketplace.Api.Tests;

public sealed class ArtifactKeeperStoreTests
{
    [Fact]
    public async Task A_brief_outage_is_retried_once()
    {
        var handler = new ScriptedHandler(HttpStatusCode.ServiceUnavailable, HttpStatusCode.OK);
        var bytes = await Store(handler).GetAsync("a/b.zip", TestContext.Current.CancellationToken);
        Assert.Equal("zip"u8.ToArray(), bytes);
        Assert.Equal(2, handler.Downloads);
    }

    [Fact]
    public async Task A_lasting_outage_is_a_503_the_caller_can_read()
    {
        var handler = new ScriptedHandler(HttpStatusCode.BadGateway, HttpStatusCode.BadGateway);
        var problem = await Assert.ThrowsAsync<ProblemException>(() => Store(handler).GetAsync("a/b.zip", TestContext.Current.CancellationToken));
        Assert.Equal(503, problem.Status);
        Assert.Contains("package store is unavailable", problem.Title);
        Assert.Equal(2, handler.Downloads);
    }

    [Fact]
    public async Task Health_checks_reuse_the_token_instead_of_logging_in_each_time()
    {
        var handler = new ScriptedHandler(HttpStatusCode.OK, HttpStatusCode.OK, HttpStatusCode.NotFound);
        var store = Store(handler);
        Assert.True(await store.CheckAsync(TestContext.Current.CancellationToken));
        Assert.True(await store.CheckAsync(TestContext.Current.CancellationToken));
        Assert.False(await store.CheckAsync(TestContext.Current.CancellationToken));
        Assert.Equal(1, handler.Logins);
    }

    private static ArtifactKeeperStore Store(HttpMessageHandler handler) => new(
        new HttpClient(handler) { BaseAddress = new Uri("http://keeper.test/") },
        Options.Create(new ArtifactKeeperOptions { Username = "svc", Password = "secret" }),
        TimeProvider.System,
        NullLogger<ArtifactKeeperStore>.Instance);

    /// <summary>Answers the login with a token and each download with the next scripted status.</summary>
    private sealed class ScriptedHandler(params HttpStatusCode[] downloads) : HttpMessageHandler
    {
        public int Downloads { get; private set; }

        public int Logins { get; private set; }

        protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
        {
            if (request.RequestUri!.AbsolutePath.EndsWith("/auth/login", StringComparison.Ordinal))
            {
                Logins++;
                return Task.FromResult(new HttpResponseMessage(HttpStatusCode.OK) { Content = new StringContent("""{"access_token":"token"}""") });
            }

            var status = downloads[Downloads++];
            return Task.FromResult(new HttpResponseMessage(status) { Content = new ByteArrayContent("zip"u8.ToArray()) });
        }
    }
}
