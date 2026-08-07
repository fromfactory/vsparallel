import { defineConfig } from "vite";

const productionConnectSource = "connect-src https://api.github.com";
const developmentConnectSource =
  "connect-src 'self' https://api.github.com ws://localhost:* ws://127.0.0.1:*";

export default defineConfig(({ command, isPreview }) => ({
  base: "/vsparallel/",
  plugins:
    command === "serve" && !isPreview
      ? [
          {
            name: "vsparallel-development-csp",
            transformIndexHtml(html) {
              return html.replace(productionConnectSource, developmentConnectSource);
            },
          },
        ]
      : [],
}));
