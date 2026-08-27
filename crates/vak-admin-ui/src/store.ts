import { createResource } from "solid-js";
import { api, retryAuthentication } from "./api";

export const useRuntime = () => {
  const [projects, projectActions] = createResource(api.projects);
  const [sessions, sessionActions] = createResource(() => api.sessions());
  const [runs, runActions] = createResource(() => api.runs());
  const [tasks, taskActions] = createResource(api.tasks);
  const [memory, memoryActions] = createResource(api.memory);
  const [inbox, inboxActions] = createResource(api.inbox);
  const [diagnostics, diagnosticsActions] = createResource(api.diagnostics);

  const retry = () => {
    retryAuthentication();
    void projectActions.refetch();
    void sessionActions.refetch();
    void runActions.refetch();
    void taskActions.refetch();
    void memoryActions.refetch();
    void inboxActions.refetch();
    void diagnosticsActions.refetch();
  };

  return { projects, sessions, runs, tasks, memory, inbox, diagnostics, retry };
};
