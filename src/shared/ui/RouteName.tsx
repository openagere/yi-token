/** 与后端 `provider_slug` 一致：空白与 `/` 折叠为 `-`。 */
export function providerSlug(name: string): string {
  return name.split(/[\s/]+/).filter(Boolean).join("-");
}

/** 对外暴露的模型名：`{Provider}/{模型}`。 */
export function routeName(provider: string, model: string): string {
  return `${providerSlug(provider)}/${model.trim()}`;
}

/** 拆出 Provider 前缀；模型名本身可以含 `/`，只按第一个分隔。 */
export function splitRoute(route: string): { provider: string; model: string } {
  const index = route.indexOf("/");
  return index > 0 ? { provider: route.slice(0, index), model: route.slice(index + 1) } : { provider: "", model: route };
}

/** 路由名：Provider 作为弱化前缀，模型名为主体。 */
export function RouteName({ route, className = "" }: { route: string; className?: string }) {
  const { provider, model } = splitRoute(route);
  return <span className={`route-name ${className}`} title={route}>
    {provider && <><span className="route-name-provider">{provider}</span><span className="route-name-separator" aria-hidden="true">/</span></>}
    <span className="route-name-model">{model}</span>
  </span>;
}
