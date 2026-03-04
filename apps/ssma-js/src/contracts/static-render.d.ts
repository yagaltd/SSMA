export type JsonSchema = Record<string, unknown>;
export interface ContractDefinition {
  version: number;
  type: string;
  owner: string;
  schema: JsonSchema;
}
export type StaticRenderContractName = 'ISLAND_HYDRATION_INITIATED' | 'ISLAND_DATA_REQUESTED' | 'ISLAND_INVALIDATED' | 'ISLAND_DATA_RETURNED' | 'ISLAND_HYDRATION_FAILED';
export interface StaticRenderContractMap {
  'ISLAND_HYDRATION_INITIATED': ContractDefinition;
  'ISLAND_DATA_REQUESTED': ContractDefinition;
  'ISLAND_INVALIDATED': ContractDefinition;
  'ISLAND_DATA_RETURNED': ContractDefinition;
  'ISLAND_HYDRATION_FAILED': ContractDefinition;
}
declare const staticRenderContracts: StaticRenderContractMap;
export { staticRenderContracts };
export default staticRenderContracts;
