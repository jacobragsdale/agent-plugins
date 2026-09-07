using System;
using Microsoft.EntityFrameworkCore.Migrations;

#nullable disable

namespace Marketplace.Api.Data.Migrations
{
    /// <inheritdoc />
    public partial class AddAccessRules : Migration
    {
        /// <inheritdoc />
        protected override void Up(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.CreateTable(
                name: "AccessRules",
                columns: table => new
                {
                    Target = table.Column<string>(type: "character varying(81)", maxLength: 81, nullable: false),
                    Users = table.Column<string[]>(type: "text[]", nullable: false),
                    Groups = table.Column<string[]>(type: "text[]", nullable: false),
                    UpdatedBy = table.Column<string>(type: "character varying(256)", maxLength: 256, nullable: false),
                    UpdatedAt = table.Column<DateTime>(type: "timestamp with time zone", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_AccessRules", x => x.Target);
                });
        }

        /// <inheritdoc />
        protected override void Down(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropTable(
                name: "AccessRules");
        }
    }
}
