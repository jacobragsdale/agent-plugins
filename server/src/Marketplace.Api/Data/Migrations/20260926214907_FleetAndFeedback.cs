using System;
using Microsoft.EntityFrameworkCore.Migrations;
using Npgsql.EntityFrameworkCore.PostgreSQL.Metadata;

#nullable disable

namespace Marketplace.Api.Data.Migrations
{
    /// <inheritdoc />
    public partial class FleetAndFeedback : Migration
    {
        /// <inheritdoc />
        protected override void Up(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropPrimaryKey(
                name: "PK_Heartbeats",
                table: "Heartbeats");

            migrationBuilder.AddColumn<string>(
                name: "Kind",
                table: "Reports",
                type: "character varying(16)",
                maxLength: 16,
                nullable: false,
                defaultValue: "problem");

            migrationBuilder.AddColumn<string>(
                name: "Note",
                table: "Reports",
                type: "character varying(2048)",
                maxLength: 2048,
                nullable: true);

            migrationBuilder.AddColumn<string>(
                name: "McpApprovedSpec",
                table: "Packages",
                type: "character varying(8192)",
                maxLength: 8192,
                nullable: true);

            migrationBuilder.AddColumn<bool>(
                name: "RevokedByAdmin",
                table: "Packages",
                type: "boolean",
                nullable: false,
                defaultValue: false);

            migrationBuilder.AddColumn<string>(
                name: "McpServersJson",
                table: "PackageVersions",
                type: "jsonb",
                nullable: true);

            migrationBuilder.AddColumn<DateTime>(
                name: "PurgedAt",
                table: "PackageVersions",
                type: "timestamp with time zone",
                nullable: true);

            migrationBuilder.AddColumn<string>(
                name: "PurgedBy",
                table: "PackageVersions",
                type: "character varying(256)",
                maxLength: 256,
                nullable: true);

            migrationBuilder.AddColumn<string>(
                name: "Device",
                table: "Heartbeats",
                type: "character varying(120)",
                maxLength: 120,
                nullable: false,
                defaultValue: "");

            migrationBuilder.AddColumn<string>(
                name: "InstalledVersionsJson",
                table: "Heartbeats",
                type: "jsonb",
                nullable: false,
                defaultValue: "{}");

            migrationBuilder.AddPrimaryKey(
                name: "PK_Heartbeats",
                table: "Heartbeats",
                columns: new[] { "Account", "Device" });

            migrationBuilder.CreateTable(
                name: "AuditEvents",
                columns: table => new
                {
                    Id = table.Column<long>(type: "bigint", nullable: false)
                        .Annotation("Npgsql:ValueGenerationStrategy", NpgsqlValueGenerationStrategy.IdentityByDefaultColumn),
                    At = table.Column<DateTime>(type: "timestamp with time zone", nullable: false),
                    Actor = table.Column<string>(type: "character varying(256)", maxLength: 256, nullable: false),
                    Action = table.Column<string>(type: "character varying(32)", maxLength: 32, nullable: false),
                    Target = table.Column<string>(type: "character varying(256)", maxLength: 256, nullable: false),
                    Detail = table.Column<string>(type: "character varying(1024)", maxLength: 1024, nullable: true)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_AuditEvents", x => x.Id);
                });

            migrationBuilder.CreateTable(
                name: "Blocks",
                columns: table => new
                {
                    Account = table.Column<string>(type: "character varying(256)", maxLength: 256, nullable: false),
                    BlockedBy = table.Column<string>(type: "character varying(256)", maxLength: 256, nullable: false),
                    BlockedAt = table.Column<DateTime>(type: "timestamp with time zone", nullable: false),
                    Reason = table.Column<string>(type: "character varying(1024)", maxLength: 1024, nullable: true)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_Blocks", x => x.Account);
                });

            migrationBuilder.CreateTable(
                name: "Notifications",
                columns: table => new
                {
                    Id = table.Column<long>(type: "bigint", nullable: false)
                        .Annotation("Npgsql:ValueGenerationStrategy", NpgsqlValueGenerationStrategy.IdentityByDefaultColumn),
                    Account = table.Column<string>(type: "character varying(256)", maxLength: 256, nullable: false),
                    Kind = table.Column<string>(type: "character varying(32)", maxLength: 32, nullable: false),
                    Text = table.Column<string>(type: "character varying(512)", maxLength: 512, nullable: false),
                    Link = table.Column<string>(type: "character varying(256)", maxLength: 256, nullable: true),
                    CreatedAt = table.Column<DateTime>(type: "timestamp with time zone", nullable: false),
                    ReadAt = table.Column<DateTime>(type: "timestamp with time zone", nullable: true)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_Notifications", x => x.Id);
                });

            migrationBuilder.CreateIndex(
                name: "IX_Reports_PackageId",
                table: "Reports",
                column: "PackageId");

            migrationBuilder.CreateIndex(
                name: "IX_Notifications_Account_Id",
                table: "Notifications",
                columns: new[] { "Account", "Id" });
        }

        /// <inheritdoc />
        protected override void Down(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropTable(
                name: "AuditEvents");

            migrationBuilder.DropTable(
                name: "Blocks");

            migrationBuilder.DropTable(
                name: "Notifications");

            migrationBuilder.DropIndex(
                name: "IX_Reports_PackageId",
                table: "Reports");

            migrationBuilder.DropPrimaryKey(
                name: "PK_Heartbeats",
                table: "Heartbeats");

            migrationBuilder.DropColumn(
                name: "Kind",
                table: "Reports");

            migrationBuilder.DropColumn(
                name: "Note",
                table: "Reports");

            migrationBuilder.DropColumn(
                name: "McpApprovedSpec",
                table: "Packages");

            migrationBuilder.DropColumn(
                name: "RevokedByAdmin",
                table: "Packages");

            migrationBuilder.DropColumn(
                name: "McpServersJson",
                table: "PackageVersions");

            migrationBuilder.DropColumn(
                name: "PurgedAt",
                table: "PackageVersions");

            migrationBuilder.DropColumn(
                name: "PurgedBy",
                table: "PackageVersions");

            migrationBuilder.DropColumn(
                name: "Device",
                table: "Heartbeats");

            migrationBuilder.DropColumn(
                name: "InstalledVersionsJson",
                table: "Heartbeats");

            migrationBuilder.AddPrimaryKey(
                name: "PK_Heartbeats",
                table: "Heartbeats",
                column: "Account");
        }
    }
}
