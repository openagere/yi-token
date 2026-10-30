import { useEffect, useState } from "react";
import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { describeError } from "@/shared/api";
import { terminalApi } from "./api";
import type { TerminalClient, TerminalDirectProfile, TerminalProfile, TerminalStatus } from "./types";

export const terminalKeys = {
  profiles: ["terminal", "profiles"] as const,
  activeEngines: ["terminal", "active-engines"] as const,
  preview: (profileKey: string) => ["terminal", "preview", profileKey] as const,
  previewDirect: (profileKey: string) => ["terminal", "preview-direct", profileKey] as const,
};

const PREVIEW_DEBOUNCE_MS = 250;

function useDebouncedValue<T>(value: T, delay: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = window.setTimeout(() => setDebounced(value), delay);
    return () => window.clearTimeout(timer);
  }, [value, delay]);
  return debounced;
}

/** 各终端客户端的接入状态；每次进入页面都重新读取，以便发现用户手动修改过的配置文件。 */
export function useTerminalProfiles() {
  return useQuery({ queryKey: terminalKeys.profiles, queryFn: terminalApi.profiles, staleTime: 0 });
}

/** Auto 模式下各终端当前选择的引擎。 */
export function useActiveEngines() {
  return useQuery({ queryKey: terminalKeys.activeEngines, queryFn: terminalApi.activeEngines, staleTime: 0 });
}

/** 切换 Auto 引擎；该操作立即生效，不改写终端配置文件。 */
export function useSetActiveEngine() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ client, modelId }: { client: TerminalClient; modelId: string | null }) =>
      terminalApi.setActiveEngine(client, modelId),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: terminalKeys.profiles });
      void queryClient.invalidateQueries({ queryKey: terminalKeys.activeEngines });
    },
  });
}

/** 把终端的实际模型切到某个已保存的 Provider 模型。立即生效，不改终端配置文件。 */
/**
 * 按草稿生成配置预览。草稿变化后先等待 250ms 再请求；等待与请求期间 `pending` 为 true，
 * 且不会返回旧草稿对应的结果。
 */
export function useTerminalDirectPreview(profile: TerminalDirectProfile, enabled: boolean) {
  const profileKey = JSON.stringify(profile);
  const settledKey = useDebouncedValue(profileKey, PREVIEW_DEBOUNCE_MS);
  const settled = settledKey === profileKey;
  const active = enabled && settled;
  const query = useQuery({
    queryKey: terminalKeys.previewDirect(settledKey),
    queryFn: () => terminalApi.previewDirect(JSON.parse(settledKey) as TerminalDirectProfile),
    enabled: active,
    placeholderData: keepPreviousData,
    staleTime: Infinity,
    refetchOnWindowFocus: false,
  });
  return {
    preview: query.data,
    error: active && query.error ? describeError(query.error) : "",
    pending: enabled && (!settled || query.isFetching),
  };
}

export function useApplyTerminalDirectConfig() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (profile: TerminalDirectProfile) => terminalApi.applyDirect(profile),
    onSuccess: (_result, profile) => {
      queryClient.setQueryData<TerminalStatus[]>(terminalKeys.profiles, (items) =>
        items?.map((item) =>
          item.client === profile.client
            ? {
                ...item,
                active: true,
                active_mode: "direct",
                exists: true,
                config_error: null,
                direct_profile: structuredClone(profile),
              }
            : item,
        ),
      );
      void queryClient.invalidateQueries({ queryKey: terminalKeys.profiles });
    },
  });
}

/** 从终端配置里去掉 yi-token。已保存的接入方式和模型选择保留，方便再次连接。 */
export function useDisconnectTerminal() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (client: TerminalClient) => terminalApi.disconnect(client),
    onSuccess: (_result, client) => {
      queryClient.setQueryData<TerminalStatus[]>(terminalKeys.profiles, (items) =>
        items?.map((item) =>
          item.client === client ? { ...item, active: false, active_mode: "none", config_error: null } : item,
        ),
      );
      void queryClient.invalidateQueries({ queryKey: terminalKeys.profiles });
    },
  });
}

/** 写入终端配置；成功后先本地更新该客户端状态，再重新读取以对齐磁盘内容。 */
export function useApplyTerminalConfig() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (profile: TerminalProfile) => terminalApi.apply(profile),
    onSuccess: (_result, profile) => {
      queryClient.setQueryData<TerminalStatus[]>(terminalKeys.profiles, (items) =>
        items?.map((item) =>
          item.client === profile.client
            ? {
                ...item,
                active: true,
                active_mode: "proxy",
                exists: true,
                config_error: null,
                profile: structuredClone(profile),
              }
            : item,
        ),
      );
      void queryClient.invalidateQueries({ queryKey: terminalKeys.profiles });
    },
  });
}
