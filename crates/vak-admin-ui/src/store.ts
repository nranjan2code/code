import { createResource } from "solid-js";
import { api } from "./api";
export const useRuntime = () => { const [projects] = createResource(api.projects); const [sessions] = createResource(api.sessions); const [runs] = createResource(api.runs); const [tasks] = createResource(api.tasks); const [memory] = createResource(api.memory); const [inbox] = createResource(api.inbox); const [diagnostics] = createResource(api.diagnostics); return { projects, sessions, runs, tasks, memory, inbox, diagnostics }; };
