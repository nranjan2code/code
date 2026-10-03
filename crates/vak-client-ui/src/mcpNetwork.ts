import * as api from "./api";

/** Whether the MCP server this approval is for may reach the network, from
 *  the settings that apply to this conversation: the project's entry when it
 *  has one, else the shared one. `undefined` when the server is not found or
 *  the settings cannot be read, because then nothing is known. */
export async function mcpServerNetwork(server: string, agent?: string): Promise<boolean | undefined> {
  try {
    const [project, shared] = await Promise.all([api.getMcpServers(agent), api.getGlobalMcpServers()]);
    const def = project.servers?.[server] ?? shared.servers?.[server];
    return def ? def.network === true : undefined;
  } catch {
    return undefined;
  }
}
