export type JsonSchema = Record<string, unknown>;
export interface ContractDefinition {
  version: number;
  type: string;
  owner: string;
  schema: JsonSchema;
}
export type OptimisticContractName = 'INTENT_BATCH' | 'EVENT_INVALIDATION' | 'PING';
export interface OptimisticContractMap {
  'INTENT_BATCH': ContractDefinition;
  'EVENT_INVALIDATION': ContractDefinition;
  'PING': ContractDefinition;
}
declare const optimisticContracts: OptimisticContractMap;
export { optimisticContracts };
export default optimisticContracts;
