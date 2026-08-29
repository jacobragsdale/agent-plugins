using System;
using Microsoft.EntityFrameworkCore.Migrations;
using Npgsql.EntityFrameworkCore.PostgreSQL.Metadata;

#nullable disable

namespace Marketplace.Api.Data.Migrations
{
    /// <inheritdoc />
    public partial class InitialCreate : Migration
    {
        /// <inheritdoc />
        protected override void Up(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.CreateTable(
                name: "Events",
                columns: table => new
                {
                    Id = table.Column<long>(type: "bigint", nullable: false)
                        .Annotation("Npgsql:ValueGenerationStrategy", NpgsqlValueGenerationStrategy.IdentityByDefaultColumn),
                    Account = table.Column<string>(type: "character varying(256)", maxLength: 256, nullable: false),
                    Kind = table.Column<string>(type: "character varying(16)", maxLength: 16, nullable: false),
                    OccurredAt = table.Column<DateTime>(type: "timestamp with time zone", nullable: false),
                    ClientVersion = table.Column<string>(type: "character varying(64)", maxLength: 64, nullable: false),
                    PackageId = table.Column<string>(type: "character varying(81)", maxLength: 81, nullable: true),
                    Version = table.Column<string>(type: "character varying(64)", maxLength: 64, nullable: true),
                    FromVersion = table.Column<string>(type: "character varying(64)", maxLength: 64, nullable: true),
                    Agents = table.Column<string[]>(type: "text[]", nullable: false),
                    ReceivedAt = table.Column<DateTime>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_Events", x => x.Id);
                });

            migrationBuilder.CreateTable(
                name: "Heartbeats",
                columns: table => new
                {
                    Account = table.Column<string>(type: "character varying(256)", maxLength: 256, nullable: false),
                    OccurredAt = table.Column<DateTime>(type: "timestamp with time zone", nullable: false),
                    ClientVersion = table.Column<string>(type: "character varying(64)", maxLength: 64, nullable: false),
                    OsBuild = table.Column<string>(type: "character varying(120)", maxLength: 120, nullable: false),
                    Agents = table.Column<string[]>(type: "text[]", nullable: false),
                    Installed = table.Column<string[]>(type: "text[]", nullable: false),
                    ChecksJson = table.Column<string>(type: "jsonb", nullable: false),
                    ReceivedAt = table.Column<DateTime>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_Heartbeats", x => x.Account);
                });

            migrationBuilder.CreateTable(
                name: "NamespaceArchives",
                columns: table => new
                {
                    Namespace = table.Column<string>(type: "character varying(16)", maxLength: 16, nullable: false),
                    Bytes = table.Column<byte[]>(type: "bytea", nullable: false),
                    Digest = table.Column<string>(type: "character varying(64)", maxLength: 64, nullable: false),
                    PackageCount = table.Column<int>(type: "integer", nullable: false),
                    GeneratedAt = table.Column<DateTime>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_NamespaceArchives", x => x.Namespace);
                });

            migrationBuilder.CreateTable(
                name: "Packages",
                columns: table => new
                {
                    Id = table.Column<int>(type: "integer", nullable: false)
                        .Annotation("Npgsql:ValueGenerationStrategy", NpgsqlValueGenerationStrategy.IdentityByDefaultColumn),
                    Namespace = table.Column<string>(type: "character varying(16)", maxLength: 16, nullable: false),
                    PackageId = table.Column<string>(type: "character varying(64)", maxLength: 64, nullable: false),
                    Name = table.Column<string>(type: "character varying(120)", maxLength: 120, nullable: false),
                    Description = table.Column<string>(type: "character varying(1024)", maxLength: 1024, nullable: false),
                    Tags = table.Column<string[]>(type: "text[]", nullable: false),
                    CreatedAt = table.Column<DateTime>(type: "timestamp with time zone", nullable: false),
                    UpdatedAt = table.Column<DateTime>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_Packages", x => x.Id);
                });

            migrationBuilder.CreateTable(
                name: "Publishers",
                columns: table => new
                {
                    Namespace = table.Column<string>(type: "character varying(16)", maxLength: 16, nullable: false),
                    Account = table.Column<string>(type: "character varying(256)", maxLength: 256, nullable: false),
                    DisplayName = table.Column<string>(type: "character varying(120)", maxLength: 120, nullable: false),
                    FirstSeenAt = table.Column<DateTime>(type: "timestamp with time zone", nullable: false),
                    LastPublishedAt = table.Column<DateTime>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_Publishers", x => x.Namespace);
                });

            migrationBuilder.CreateTable(
                name: "Reports",
                columns: table => new
                {
                    Id = table.Column<long>(type: "bigint", nullable: false)
                        .Annotation("Npgsql:ValueGenerationStrategy", NpgsqlValueGenerationStrategy.IdentityByDefaultColumn),
                    Account = table.Column<string>(type: "character varying(256)", maxLength: 256, nullable: false),
                    PackageId = table.Column<string>(type: "character varying(81)", maxLength: 81, nullable: false),
                    Reason = table.Column<string>(type: "character varying(2048)", maxLength: 2048, nullable: false),
                    CreatedAt = table.Column<DateTime>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_Reports", x => x.Id);
                });

            migrationBuilder.CreateTable(
                name: "PackageVersions",
                columns: table => new
                {
                    Id = table.Column<int>(type: "integer", nullable: false)
                        .Annotation("Npgsql:ValueGenerationStrategy", NpgsqlValueGenerationStrategy.IdentityByDefaultColumn),
                    PackageId = table.Column<int>(type: "integer", nullable: false),
                    Version = table.Column<string>(type: "character varying(64)", maxLength: 64, nullable: false),
                    StoragePath = table.Column<string>(type: "character varying(512)", maxLength: 512, nullable: false),
                    ArchiveDigest = table.Column<string>(type: "character varying(64)", maxLength: 64, nullable: false),
                    SizeBytes = table.Column<long>(type: "bigint", nullable: false),
                    ManifestJson = table.Column<string>(type: "jsonb", nullable: false),
                    ComponentKinds = table.Column<string[]>(type: "text[]", nullable: false),
                    PublishedBy = table.Column<string>(type: "character varying(256)", maxLength: 256, nullable: false),
                    PublishedAt = table.Column<DateTime>(type: "timestamp with time zone", nullable: false),
                    Changelog = table.Column<string>(type: "character varying(4096)", maxLength: 4096, nullable: true),
                    Yanked = table.Column<bool>(type: "boolean", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_PackageVersions", x => x.Id);
                    table.ForeignKey(
                        name: "FK_PackageVersions_Packages_PackageId",
                        column: x => x.PackageId,
                        principalTable: "Packages",
                        principalColumn: "Id",
                        onDelete: ReferentialAction.Cascade);
                });

            migrationBuilder.CreateIndex(
                name: "IX_Events_Account_Kind_OccurredAt_PackageId",
                table: "Events",
                columns: new[] { "Account", "Kind", "OccurredAt", "PackageId" },
                unique: true);

            migrationBuilder.CreateIndex(
                name: "IX_Events_Kind_PackageId_OccurredAt",
                table: "Events",
                columns: new[] { "Kind", "PackageId", "OccurredAt" });

            migrationBuilder.CreateIndex(
                name: "IX_PackageVersions_PackageId_Version",
                table: "PackageVersions",
                columns: new[] { "PackageId", "Version" },
                unique: true);

            migrationBuilder.CreateIndex(
                name: "IX_Packages_Namespace_PackageId",
                table: "Packages",
                columns: new[] { "Namespace", "PackageId" },
                unique: true);

            migrationBuilder.CreateIndex(
                name: "IX_Publishers_Account",
                table: "Publishers",
                column: "Account");
        }

        /// <inheritdoc />
        protected override void Down(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropTable(
                name: "Events");

            migrationBuilder.DropTable(
                name: "Heartbeats");

            migrationBuilder.DropTable(
                name: "NamespaceArchives");

            migrationBuilder.DropTable(
                name: "PackageVersions");

            migrationBuilder.DropTable(
                name: "Publishers");

            migrationBuilder.DropTable(
                name: "Reports");

            migrationBuilder.DropTable(
                name: "Packages");
        }
    }
}
