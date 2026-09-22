/** @type {import("prettier").Config} */
const config = {
  printWidth: 200,
  trailingComma: "none",
  objectWrap: "collapse",
  // Parse Angular template syntax rather than plain HTML.
  overrides: [{ files: "website/src/**/*.html", options: { parser: "angular" } }]
};

export default config;
