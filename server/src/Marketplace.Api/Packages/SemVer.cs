using System.Text.RegularExpressions;

namespace Marketplace.Api.Packages;

/// <summary>A release version, <c>major.minor.patch</c>. The marketplace takes no pre-releases, so every version is comparable.</summary>
public sealed partial record SemVer(int Major, int Minor, int Patch) : IComparable<SemVer>
{
    public static bool TryParse(string? text, out SemVer version)
    {
        version = null!;
        var match = Pattern().Match(text?.Trim() ?? string.Empty);
        if (!match.Success)
        {
            return false;
        }

        version = new SemVer(int.Parse(match.Groups["major"].Value), int.Parse(match.Groups["minor"].Value), int.Parse(match.Groups["patch"].Value));
        return true;
    }

    public static SemVer Parse(string text) =>
        TryParse(text, out var version) ? version : throw new FormatException($"{text} is not a semantic version.");

    public int CompareTo(SemVer? other) => other is null ? 1 : (Major, Minor, Patch).CompareTo((other.Major, other.Minor, other.Patch));

    public override string ToString() => $"{Major}.{Minor}.{Patch}";

    [GeneratedRegex(@"^(?<major>0|[1-9]\d{0,8})\.(?<minor>0|[1-9]\d{0,8})\.(?<patch>0|[1-9]\d{0,8})$")]
    private static partial Regex Pattern();
}
