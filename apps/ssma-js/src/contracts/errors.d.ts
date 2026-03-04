export type JsonSchema = Record<string, unknown>;
export interface ContractDefinition {
  version: number;
  type: string;
  owner: string;
  schema: JsonSchema;
}
export type ErrorsContractName = 'ERROR_FRAME';
export interface ErrorsContractMap {
  'ERROR_FRAME': ContractDefinition;
}
declare const errorsContracts: ErrorsContractMap;
export { errorsContracts };
export default errorsContracts;
