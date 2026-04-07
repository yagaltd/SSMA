export type JsonSchema = Record<string, unknown>;
export interface ContractDefinition {
  version: number;
  type: string;
  owner: string;
  schema: JsonSchema;
}
export type ChannelsContractName = 'CHANNEL_SUBSCRIBE' | 'CHANNEL_RESYNC' | 'CHANNEL_UNSUBSCRIBE' | 'CHANNEL_COMMAND';
export interface ChannelsContractMap {
  'CHANNEL_SUBSCRIBE': ContractDefinition;
  'CHANNEL_RESYNC': ContractDefinition;
  'CHANNEL_UNSUBSCRIBE': ContractDefinition;
  'CHANNEL_COMMAND': ContractDefinition;
}
declare const channelsContracts: ChannelsContractMap;
export { channelsContracts };
export default channelsContracts;
