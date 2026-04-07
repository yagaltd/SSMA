export type JsonSchema = Record<string, unknown>;
export interface ContractDefinition {
  version: number;
  type: string;
  owner: string;
  schema: JsonSchema;
}
export type LogsContractName = 'INTENT_LOG_BATCH' | 'LOG_EVENT_RECEIVED' | 'LOG_EVENT_PERSISTED';
export interface LogsContractMap {
  'INTENT_LOG_BATCH': ContractDefinition;
  'LOG_EVENT_RECEIVED': ContractDefinition;
  'LOG_EVENT_PERSISTED': ContractDefinition;
}
declare const logsContracts: LogsContractMap;
export { logsContracts };
export default logsContracts;
