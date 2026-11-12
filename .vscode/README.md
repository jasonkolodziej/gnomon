# Managing MCPs

> [!TIP]
> This [guide](https://code.visualstudio.com/docs/agent-customization/mcp-servers#_quickstart-use-an-mcp-server-in-chat) provides the necessary configuration for using MCP, via [`.vscode/mcp.json`](./mcp.json) in Visual Studio Code with GitHub Copilot.

> [!IMPORTANT]
> Read configuration documentation for [Cloudflare MCP Servers](https://developers.cloudflare.com/agents/model-context-protocol/cloudflare/servers-for-cloudflare/)
> Per [Github documentation](https://github.com/cloudflare/mcp#option-2-api-token), Both user tokens and account tokens are supported. For account tokens, include the Account Resources : Read permission so the server can auto-detect your account ID.

## MCP Config for cloud agent on `github.com`

> [!NOTE]
> With MCP, you can extend the capabilities of Copilot cloud agent by connecting it to other tools and services. The GitHub and Playwright MCP servers are enabled by default.

> [!TIP] 
> You can configure your own MCP servers by adding JSON configuration [here in `<org>/<repo>/settings`](`https://github.com/<ORG>/<REPO>/settings/copilot/coding_agent`). MCP servers can optionally access secrets defined in the repository's copilot environment.
> [Learn more on cloud agent configuration](https://docs.github.com/en/enterprise-cloud@latest/copilot/how-tos/copilot-on-github/customize-copilot/customize-cloud-agent/extend-cloud-agent-with-mcp)

## Example MCP Config

```jsonl
 {
    // ~/.copilot/mcp-config.json || on github repo
   "mcpServers": {
     "github-mcp-server": {
       "type": "http",
       // Remove "/readonly" to enable wider access to all tools.
       // Then, use the "X-MCP-Toolsets" header to specify which toolsets you'd like to include.
       // Use the "tools" field to select individual tools from the toolsets.
       "url": "https://api.githubcopilot.com/mcp/readonly",
       "tools": ["*"],
       "headers": {
         "X-MCP-Toolsets": "repos,issues,users,pull_requests,code_security,secret_protection,actions,web_search"
       }
     },
        "context7": {
      "type": "http",
      "url": "https://mcp.context7.com/mcp",
      "headers": {
        "CONTEXT7_API_KEY": "COPILOT_MCP_CONTEXT7_API_KEY"
      },
      "tools": ["query-docs", "resolve-library-id"]
    }
   }
 }
```

> [!IMPORTANT]
> When migrating from VS Code MCP config, do not copy `${input:...}` placeholders directly. Use environment-backed values instead, for example `${COPILOT_MCP_CONTEXT7_API_KEY}`, `${COPILOT_MCP_SHADCN_SVELTE}`, or `${COPILOT_MCP_CLOUDFLARE_API_TOKEN}`. Keep the original `.vscode/mcp.json` until the migrated config is validated in the selected Agent Host and cleanup is explicitly approved.

