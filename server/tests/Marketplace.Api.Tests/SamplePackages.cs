using System.IO.Compression;
using System.Net.Http.Headers;
using System.Text;

namespace Marketplace.Api.Tests;

public static class SamplePackages
{
    public static byte[] SkillPackage(string ns, string packageId, string description = "Says hello when asked to greet someone.", string? extraFile = null)
    {
        var manifest = $$"""
            {
              "version": 2,
              "source": { "id": "{{ns}}", "name": "{{ns}}", "description": "Test packages." },
              "packages": [
                {
                  "id": "{{packageId}}",
                  "name": "{{packageId}} skill",
                  "description": "{{description}}",
                  "components": [ { "kind": "skill", "path": "skills/{{packageId}}" } ]
                }
              ]
            }
            """;
        var skill = $"""
            ---
            name: {packageId}
            description: {description}
            ---

            # {packageId}

            Reply with a friendly greeting.
            """;
        var files = new Dictionary<string, string>
        {
            ["agent-plugins.json"] = manifest,
            [$"skills/{packageId}/SKILL.md"] = skill,
        };
        if (extraFile is not null)
        {
            files[$"skills/{packageId}/{extraFile}"] = "extra content\n";
        }

        return Zip(files);
    }

    public static byte[] Zip(IReadOnlyDictionary<string, string> files, string prefix = "")
    {
        using var output = new MemoryStream();
        using (var zip = new ZipArchive(output, ZipArchiveMode.Create, leaveOpen: true))
        {
            foreach (var (name, content) in files)
            {
                var entry = zip.CreateEntry(prefix + name);
                using var stream = entry.Open();
                stream.Write(Encoding.UTF8.GetBytes(content));
            }
        }

        return output.ToArray();
    }

    public static MultipartFormDataContent PublishForm(byte[] archive, string version, string? tags = null, string? changelog = null)
    {
        var form = new MultipartFormDataContent();
        var file = new ByteArrayContent(archive);
        file.Headers.ContentType = new MediaTypeHeaderValue("application/zip");
        form.Add(file, "archive", "package.zip");
        form.Add(new StringContent(version), "version");
        if (tags is not null)
        {
            form.Add(new StringContent(tags), "tags");
        }

        if (changelog is not null)
        {
            form.Add(new StringContent(changelog), "changelog");
        }

        return form;
    }
}
