using Marketplace.Api.Auth;
using Xunit;

namespace Marketplace.Api.Tests;

public sealed class AccountMatchingTests
{
    [Theory]
    [InlineData("CORP\\jane", "CORP\\jane", true)]
    [InlineData("CORP\\jane", "jane@CORP.EXAMPLE.COM", true)]
    [InlineData("jane@corp.example.com", "CORP\\jane", true)]
    [InlineData("jane", "EUROPE\\jane", true)]
    [InlineData("CORP\\jane", "EUROPE\\jane", false)]
    [InlineData("CORP\\jane", "jane@EUROPE.EXAMPLE.COM", false)]
    [InlineData("CORP\\jane", "jane", false)]
    [InlineData("CORP\\jane", "CORP\\janet", false)]
    public void A_domain_entry_matches_either_spelling_of_that_domain_only(string entry, string account, bool matches) =>
        Assert.Equal(matches, IdentityResolver.EntryMatches(entry, account));
}
