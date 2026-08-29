using System.Text.RegularExpressions;

namespace Marketplace.Api.Packages;

/// <summary>Enough of semantic versioning to order marketplace versions.</summary>
public sealed partial record SemVer(int Major, int Minor, int Patch, string? PreRelease) : IComparable<SemVer>
{
    public static bool TryParse(string? text, out SemVer version)
    {
        version = null!;
        if (text is null)
        {
            return false;
        }

        var match = Pattern().Match(text.Trim());
        if (!match.Success)
        {
            return false;
        }

        version = new SemVer(
            int.Parse(match.Groups["major"].Value),
            int.Parse(match.Groups["minor"].Value),
            int.Parse(match.Groups["patch"].Value),
            match.Groups["pre"].Success ? match.Groups["pre"].Value : null);
        return true;
    }

    public static SemVer Parse(string text) =>
        TryParse(text, out var version) ? version : throw new FormatException($"{text} is not a semantic version.");

    public int CompareTo(SemVer? other)
    {
        if (other is null)
        {
            return 1;
        }

        var core = (Major, Minor, Patch).CompareTo((other.Major, other.Minor, other.Patch));
        if (core != 0)
        {
            return core;
        }

        return (PreRelease, other.PreRelease) switch
        {
            (null, null) => 0,
            (null, _) => 1,
            (_, null) => -1,
            _ => string.CompareOrdinal(PreRelease, other.PreRelease),
        };
    }

    public override string ToString() => PreRelease is null ? $"{Major}.{Minor}.{Patch}" : $"{Major}.{Minor}.{Patch}-{PreRelease}";

    [GeneratedRegex(@"^(?<major>0|[1-9]\d{0,8})\.(?<minor>0|[1-9]\d{0,8})\.(?<patch>0|[1-9]\d{0,8})(?:-(?<pre>[0-9A-Za-z][0-9A-Za-z.-]{0,40}))?$")]
    private static partial Regex Pattern();
}
