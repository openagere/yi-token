import type { StandardModel } from "@/features/models";
import type { ProviderView } from "@/features/providers";
import { modelBrand, routeName } from "@/shared/ui";

export type UsageModelFormatter = (routeId: string) => string;
export type UsageModelBrandResolver = (routeId: string) => string | undefined;

export function createUsageModelFormatter(providers: readonly ProviderView[]): UsageModelFormatter {
  const names = new Map(providers.map((provider) => [provider.id, provider.name]));
  return (routeId) => {
    const separator = routeId.indexOf("/");
    if (separator <= 0 || separator === routeId.length - 1) return routeId;
    const name = names.get(routeId.slice(0, separator));
    return name ? routeName(name, routeId.slice(separator + 1)) : routeId;
  };
}

export function createUsageBrandResolver(providers: readonly ProviderView[], standardModels: readonly StandardModel[]): UsageModelBrandResolver {
  const standards = new Map(standardModels.map((standard) => [standard.id, standard]));
  const brands = new Map(providers.flatMap((provider) => provider.models.map((binding) => {
    const standard = binding.standard_model_id ? standards.get(binding.standard_model_id) : undefined;
    return [binding.route_id, standard?.brand || (standard ? modelBrand(standard.name) : undefined) || modelBrand(binding.name)] as const;
  })));
  return (routeId) => brands.get(routeId) || modelBrand(routeId);
}
