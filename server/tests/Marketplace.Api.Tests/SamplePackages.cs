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

    /// <summary>A skill plus an MCP server in one package: the kind of version that always waits for review.</summary>
    public static byte[] SkillAndMcpPackage(string ns, string packageId, string description)
    {
        var manifest = $$"""
            {
              "version": 2,
              "source": { "id": "{{ns}}", "name": "{{ns}}", "description": "Test packages." },
              "packages": [
                {
                  "id": "{{packageId}}",
                  "name": "{{packageId}} with a server",
                  "description": "{{description}}",
                  "components": [
                    { "kind": "skill", "id": "{{packageId}}", "path": "skills/{{packageId}}" },
                    { "kind": "mcpServer", "id": "database", "path": "mcp/database.json" }
                  ]
                }
              ]
            }
            """;
        return Zip(new Dictionary<string, string>
        {
            ["agent-plugins.json"] = manifest,
            [$"skills/{packageId}/SKILL.md"] = $"---\nname: {packageId}\ndescription: {description}\n---\n\nUse the database server.\n",
            ["mcp/database.json"] = """{ "$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json", "mcpServers": { "database": { "type": "stdio", "command": "node", "args": ["server.js"] } } }""",
        });
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

    /// <summary>A browser-style upload: every file with its relative path, plus optional form fields.</summary>
    public static MultipartFormDataContent UploadForm(string version, IReadOnlyDictionary<string, string> files, IReadOnlyDictionary<string, string>? fields = null)
    {
        var form = new MultipartFormDataContent();
        foreach (var (path, content) in files)
        {
            form.Add(new ByteArrayContent(Encoding.UTF8.GetBytes(content)), "files", Path.GetFileName(path));
            form.Add(new StringContent(path), "paths");
        }

        form.Add(new StringContent(version), "version");
        foreach (var (name, value) in fields ?? new Dictionary<string, string>())
        {
            form.Add(new StringContent(value), name);
        }

        return form;
    }

    public static string Skill(string name, string description, string body = "Follow these steps.") =>
        $"---\nname: {name}\ndescription: {description}\n---\n\n{body}\n";

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
