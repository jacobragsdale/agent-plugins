using System;
using Microsoft.EntityFrameworkCore.Migrations;

#nullable disable

namespace Marketplace.Api.Data.Migrations
{
    /// <inheritdoc />
    public partial class AddReviewState : Migration
    {
        /// <inheritdoc />
        protected override void Up(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.AddColumn<DateTime>(
                name: "ResolvedAt",
                table: "Reports",
                type: "timestamp with time zone",
                nullable: true);

            migrationBuilder.AddColumn<string>(
                name: "ResolvedBy",
                table: "Reports",
                type: "character varying(256)",
                maxLength: 256,
                nullable: true);

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
                // Versions published before the review queue existed were already live.
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

            migrationBuilder.AddColumn<string[]>(
                name: "Tags",
                table: "PackageVersions",
                type: "text[]",
                nullable: false,
                defaultValue: new string[0]);
        }

        /// <inheritdoc />
        protected override void Down(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropColumn(
                name: "ResolvedAt",
                table: "Reports");

            migrationBuilder.DropColumn(
                name: "ResolvedBy",
                table: "Reports");

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

            migrationBuilder.DropColumn(
                name: "Tags",
                table: "PackageVersions");
        }
    }
}
