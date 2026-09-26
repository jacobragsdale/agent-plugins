using System;
using Microsoft.EntityFrameworkCore.Migrations;
using Npgsql.EntityFrameworkCore.PostgreSQL.Metadata;

#nullable disable

namespace Marketplace.Api.Data.Migrations
{
    /// <inheritdoc />
    public partial class SelfService : Migration
    {
        /// <inheritdoc />
        protected override void Up(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.AddColumn<string>(
                name: "Kind",
                table: "Publishers",
                type: "character varying(16)",
                maxLength: 16,
                nullable: false,
                defaultValue: "Personal");

            migrationBuilder.AddColumn<DateTime>(
                name: "McpApprovedAt",
                table: "Packages",
                type: "timestamp with time zone",
                nullable: true);

            migrationBuilder.AddColumn<string>(
                name: "McpApprovedBy",
                table: "Packages",
                type: "character varying(256)",
                maxLength: 256,
                nullable: true);

            migrationBuilder.AddColumn<string>(
                name: "McpDeclineNote",
                table: "Packages",
                type: "character varying(2048)",
                maxLength: 2048,
                nullable: true);

            migrationBuilder.AddColumn<DateTime>(
                name: "RevokedAt",
                table: "Packages",
                type: "timestamp with time zone",
                nullable: true);

            migrationBuilder.AddColumn<string>(
                name: "RevokedBy",
                table: "Packages",
                type: "character varying(256)",
                maxLength: 256,
                nullable: true);

            migrationBuilder.AddColumn<string[]>(
                name: "Teams",
                table: "AccessRules",
                type: "text[]",
                nullable: false,
                defaultValue: new string[0]);

            migrationBuilder.AddColumn<string>(
                name: "Visibility",
                table: "AccessRules",
                type: "character varying(16)",
                maxLength: 16,
                nullable: false,
                defaultValue: "Private");

            // Existing rules were allowlists, so they stay private. Only official is not a person's: no
            // deployment configured AD-group teams. Versions stop having a review state: pending ones go
            // live, rejected ones are withdrawn, and a package whose MCP server an admin approved (or
            // published) keeps that approval for the public.
            migrationBuilder.Sql("""
                UPDATE "Publishers" SET "Kind" = 'Official' WHERE "Namespace" = 'official';
                UPDATE "Packages" AS p SET "McpApprovedBy" = v."ReviewedBy", "McpApprovedAt" = v."ReviewedAt"
                  FROM "PackageVersions" AS v
                  WHERE v."PackageId" = p."Id" AND v."ReviewState" = 'Approved' AND v."ReviewedBy" IS NOT NULL
                    AND 'mcpServer' = ANY (v."ComponentKinds");
                UPDATE "PackageVersions" SET "Yanked" = TRUE WHERE "ReviewState" = 'Rejected';
                """);

            migrationBuilder.DropColumn(
                name: "ReviewNote",
                table: "PackageVersions");

            migrationBuilder.DropColumn(
                name: "ReviewState",
                table: "PackageVersions");

            migrationBuilder.DropColumn(
                name: "ReviewedAt",
                table: "PackageVersions");

            migrationBuilder.DropColumn(
                name: "ReviewedBy",
                table: "PackageVersions");

            migrationBuilder.CreateTable(
                name: "Bundles",
                columns: table => new
                {
                    Namespace = table.Column<string>(type: "character varying(16)", maxLength: 16, nullable: false),
                    BundleId = table.Column<string>(type: "character varying(64)", maxLength: 64, nullable: false),
                    Name = table.Column<string>(type: "character varying(120)", maxLength: 120, nullable: false),
                    Description = table.Column<string>(type: "character varying(1024)", maxLength: 1024, nullable: false),
                    Members = table.Column<string[]>(type: "text[]", nullable: false),
                    UpdatedBy = table.Column<string>(type: "character varying(256)", maxLength: 256, nullable: false),
                    CreatedAt = table.Column<DateTime>(type: "timestamp with time zone", nullable: false),
                    UpdatedAt = table.Column<DateTime>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_Bundles", x => new { x.Namespace, x.BundleId });
                });

            migrationBuilder.CreateTable(
                name: "Links",
                columns: table => new
                {
                    Code = table.Column<string>(type: "character varying(32)", maxLength: 32, nullable: false),
                    Kind = table.Column<string>(type: "character varying(16)", maxLength: 16, nullable: false),
                    Target = table.Column<string>(type: "character varying(81)", maxLength: 81, nullable: false),
                    CreatedBy = table.Column<string>(type: "character varying(256)", maxLength: 256, nullable: false),
                    CreatedAt = table.Column<DateTime>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_Links", x => x.Code);
                });

            migrationBuilder.CreateTable(
                name: "People",
                columns: table => new
                {
                    Account = table.Column<string>(type: "character varying(256)", maxLength: 256, nullable: false),
                    DisplayName = table.Column<string>(type: "character varying(120)", maxLength: 120, nullable: false),
                    LastSeenAt = table.Column<DateTime>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_People", x => x.Account);
                });

            migrationBuilder.CreateTable(
                name: "Suggestions",
                columns: table => new
                {
                    Id = table.Column<long>(type: "bigint", nullable: false)
                        .Annotation("Npgsql:ValueGenerationStrategy", NpgsqlValueGenerationStrategy.IdentityByDefaultColumn),
                    PackageId = table.Column<int>(type: "integer", nullable: false),
                    StoragePath = table.Column<string>(type: "character varying(512)", maxLength: 512, nullable: false),
                    ArchiveDigest = table.Column<string>(type: "character varying(64)", maxLength: 64, nullable: false),
                    SizeBytes = table.Column<long>(type: "bigint", nullable: false),
                    ManifestJson = table.Column<string>(type: "jsonb", nullable: false),
                    ComponentKinds = table.Column<string[]>(type: "text[]", nullable: false),
                    BaseVersion = table.Column<string>(type: "character varying(64)", maxLength: 64, nullable: true),
                    Message = table.Column<string>(type: "character varying(4096)", maxLength: 4096, nullable: false),
                    SuggestedBy = table.Column<string>(type: "character varying(256)", maxLength: 256, nullable: false),
                    SuggestedAt = table.Column<DateTime>(type: "timestamp with time zone", nullable: false),
                    State = table.Column<string>(type: "character varying(16)", maxLength: 16, nullable: false),
                    DecidedBy = table.Column<string>(type: "character varying(256)", maxLength: 256, nullable: true),
                    DecidedAt = table.Column<DateTime>(type: "timestamp with time zone", nullable: true),
                    DecisionNote = table.Column<string>(type: "character varying(2048)", maxLength: 2048, nullable: true),
                    AcceptedVersion = table.Column<string>(type: "character varying(64)", maxLength: 64, nullable: true)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_Suggestions", x => x.Id);
                    table.ForeignKey(
                        name: "FK_Suggestions_Packages_PackageId",
                        column: x => x.PackageId,
                        principalTable: "Packages",
                        principalColumn: "Id",
                        onDelete: ReferentialAction.Cascade);
                });

            migrationBuilder.CreateTable(
                name: "TeamMembers",
                columns: table => new
                {
                    Namespace = table.Column<string>(type: "character varying(16)", maxLength: 16, nullable: false),
                    Account = table.Column<string>(type: "character varying(256)", maxLength: 256, nullable: false),
                    IsOwner = table.Column<bool>(type: "boolean", nullable: false),
                    JoinedAt = table.Column<DateTime>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_TeamMembers", x => new { x.Namespace, x.Account });
                });

            migrationBuilder.CreateIndex(
                name: "IX_Links_Kind_Target",
                table: "Links",
                columns: new[] { "Kind", "Target" },
                unique: true);

            migrationBuilder.CreateIndex(
                name: "IX_Suggestions_PackageId",
                table: "Suggestions",
                column: "PackageId");

            migrationBuilder.CreateIndex(
                name: "IX_Suggestions_State",
                table: "Suggestions",
                column: "State");

            migrationBuilder.CreateIndex(
                name: "IX_TeamMembers_Account",
                table: "TeamMembers",
                column: "Account");
        }

        /// <inheritdoc />
        protected override void Down(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropTable(
                name: "Bundles");

            migrationBuilder.DropTable(
                name: "Links");

            migrationBuilder.DropTable(
                name: "People");

            migrationBuilder.DropTable(
                name: "Suggestions");

            migrationBuilder.DropTable(
                name: "TeamMembers");

            migrationBuilder.DropColumn(
                name: "Kind",
                table: "Publishers");

            migrationBuilder.DropColumn(
                name: "McpApprovedAt",
                table: "Packages");

            migrationBuilder.DropColumn(
                name: "McpApprovedBy",
                table: "Packages");

            migrationBuilder.DropColumn(
                name: "McpDeclineNote",
                table: "Packages");

            migrationBuilder.DropColumn(
                name: "RevokedAt",
                table: "Packages");

            migrationBuilder.DropColumn(
                name: "RevokedBy",
                table: "Packages");

            migrationBuilder.DropColumn(
                name: "Teams",
                table: "AccessRules");

            migrationBuilder.DropColumn(
                name: "Visibility",
                table: "AccessRules");

            migrationBuilder.AddColumn<string>(
                name: "ReviewNote",
                table: "PackageVersions",
                type: "character varying(2048)",
                maxLength: 2048,
                nullable: true);

            migrationBuilder.AddColumn<string>(
                name: "ReviewState",
                table: "PackageVersions",
                type: "character varying(16)",
                maxLength: 16,
                nullable: false,
                defaultValue: "Approved");

            migrationBuilder.AddColumn<DateTime>(
                name: "ReviewedAt",
                table: "PackageVersions",
                type: "timestamp with time zone",
                nullable: true);

            migrationBuilder.AddColumn<string>(
                name: "ReviewedBy",
                table: "PackageVersions",
                type: "character varying(256)",
                maxLength: 256,
                nullable: true);
        }
    }
}
