import { useEffect, useRef } from "react";
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { describeError } from "@/shared/api";
import { notify } from "@/shared/notify";
import { sessionsApi } from "./api";
import type { Client } from "@/generated";
import type { SessionFilter, TrajectorySettings } from "./types";

export const SESSION_PAGE_SIZE = 50;
export const TRAJECTORY_PAGE_TURNS = 20;
const CHANGES_INTERVAL = 2000;

export const sessionKeys = {
  all: ["sessions"] as const,
  lists: ["sessions", "list"] as const,
  list: (filter: SessionFilter) => ["sessions", "list", filter] as const,
  detail: (id: string | null) => ["sessions", "detail", id] as const,
  trajectoryAll: (sessionId: string | null) => ["sessions", "trajectory", sessionId] as const,
  trajectory: (sessionId: string | null, threadId: string | null) => ["sessions", "trajectory", sessionId, threadId] as const,
  stepContext: (stepId: string | null) => ["sessions", "step", stepId] as const,
  blob: (hash: string | null) => ["sessions", "blob", hash] as const,
  changes: ["sessions", "changes"] as const,
  status: ["sessions", "status"] as const,
};

/** 会话列表：按 `last_at` 倒序的键集分页，`pages[0]` 为最新一页。 */
export function useSessionList(filter: SessionFilter) {
  return useInfiniteQuery({
    queryKey: sessionKeys.list(filter),
    queryFn: ({ pageParam }) => sessionsApi.list({ ...filter, cursor: pageParam, limit: SESSION_PAGE_SIZE }),
    initialPageParam: null as string | null,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
  });
}

export function useSessionDetail(id: string | null) {
  return useQuery({
    queryKey: sessionKeys.detail(id),
    queryFn: () => sessionsApi.get(id ?? ""),
    enabled: id !== null,
  });
}

/**
 * 轨迹回合：`pages[0]` 为最近的回合，之后的页是更早的回合；每页内部按回合号升序。
 * 向前翻页使用上一页最早回合的 `turn_no` 作为 `before_turn`。
 */
export function useTrajectory(sessionId: string | null, threadId: string | null) {
  return useInfiniteQuery({
    queryKey: sessionKeys.trajectory(sessionId, threadId),
    queryFn: ({ pageParam }) =>
      sessionsApi.trajectory({
        session_id: sessionId ?? "",
        thread_id: threadId,
        before_turn: pageParam,
        limit_turns: TRAJECTORY_PAGE_TURNS,
      }),
    enabled: sessionId !== null,
    initialPageParam: null as number | null,
    getNextPageParam: (lastPage) => {
      if (!lastPage.has_older) return undefined;
      const oldest = lastPage.turns[0];
      return oldest?.turn_no ?? undefined;
    },
  });
}

/**
 * 每 2 秒用 `since_seq` 轮询变化；有变化时让受影响的会话详情与轨迹失效，列表也一并失效。
 * 只在 `active` 为真时轮询。
 */
export function useSessionChanges(active: boolean) {
  const client = useQueryClient();
  const sinceSeq = useRef(0);
  const query = useQuery({
    queryKey: sessionKeys.changes,
    queryFn: async () => {
      const changes = await sessionsApi.changes(sinceSeq.current);
      sinceSeq.current = changes.seq;
      return changes;
    },
    enabled: active,
    refetchInterval: active ? CHANGES_INTERVAL : false,
    staleTime: 0,
  });
  const { dataUpdatedAt, data } = query;

  useEffect(() => {
    if (!data?.session_ids.length) return;
    void client.invalidateQueries({ queryKey: sessionKeys.lists });
    for (const id of data.session_ids) {
      void client.invalidateQueries({ queryKey: sessionKeys.detail(id) });
      void client.invalidateQueries({ queryKey: sessionKeys.trajectoryAll(id) });
    }
    // `dataUpdatedAt` changes on every successful poll, including ones that repeat the same ids.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, dataUpdatedAt]);

  return query;
}

/** 单步的完整上下文；步骤 ID 为空时不请求。 */
export function useStepContext(stepId: string | null) {
  return useQuery({
    queryKey: sessionKeys.stepContext(stepId),
    queryFn: () => sessionsApi.stepContext(stepId ?? ""),
    enabled: stepId !== null,
  });
}

/** 内容寻址的正文块不会变化，因此永不过期。 */
export function useBlob(hash: string | null) {
  return useQuery({
    queryKey: sessionKeys.blob(hash),
    queryFn: () => sessionsApi.blob(hash ?? ""),
    enabled: hash !== null,
    staleTime: Infinity,
  });
}

export function useTrajectoryStatus() {
  return useQuery({ queryKey: sessionKeys.status, queryFn: sessionsApi.status });
}

/** 删除会话后让列表重新拉取，并丢弃该会话的详情与轨迹缓存。 */
export function useDeleteSession() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => sessionsApi.remove(id),
    onSuccess: (_result, id) => {
      client.removeQueries({ queryKey: sessionKeys.detail(id) });
      client.removeQueries({ queryKey: sessionKeys.trajectoryAll(id) });
      return client.invalidateQueries({ queryKey: sessionKeys.lists });
    },
  });
}

/** 清空某终端（`null` 为全部）的会话与轨迹；返回删除的会话数。 */
export function useClearSessions() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (target: Client | null) => sessionsApi.clear(target),
    onSuccess: () => client.invalidateQueries({ queryKey: sessionKeys.all }),
  });
}

/** 重建轨迹库：旧文件由后端重命名保留，随后刷新状态与会话。 */
export function useRebuildTrajectory() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: () => sessionsApi.rebuild(),
    onSuccess: () => client.invalidateQueries({ queryKey: sessionKeys.all }),
    onError: (error) => notify(describeError(error), "error"),
  });
}

/** 保存完整的轨迹设置（读出后修改字段，再整体写回）。 */
export function useSaveTrajectorySettings() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (settings: TrajectorySettings) => sessionsApi.saveSettings(settings),
    onSuccess: () => client.invalidateQueries({ queryKey: sessionKeys.status }),
    onError: (error) => notify(describeError(error), "error"),
  });
}
