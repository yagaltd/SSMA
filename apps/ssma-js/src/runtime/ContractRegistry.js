import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import Ajv from "ajv";
import addFormats from "ajv-formats";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const protocolContractsDir = path.resolve(
  __dirname,
  "../../../packages/ssma-protocol/contracts",
);
const legacyContractsDir = path.resolve(__dirname, "../contracts");

const ajv = new Ajv({ allErrors: true, strict: false });
addFormats(ajv);
const contractCache = new Map();
const validatorCache = new Map();

function loadContracts() {
  const loadDir = (dirPath) => {
    const files = fs.existsSync(dirPath)
      ? fs.readdirSync(dirPath).filter((file) => file.endsWith(".json"))
      : [];
    for (const file of files) {
      const name = path.basename(file, ".json");
      const data = JSON.parse(
        fs.readFileSync(path.join(dirPath, file), "utf-8"),
      );
      contractCache.set(name, data);
    }
  };

  loadDir(legacyContractsDir);
  loadDir(protocolContractsDir);
}

loadContracts();

export function getContracts() {
  return contractCache;
}

export function getContract(contractGroup, contractName) {
  const group = contractCache.get(contractGroup);
  if (!group) return null;
  return group[contractName] || null;
}

export function validateContract(contractGroup, contractName, payload) {
  const contract = getContract(contractGroup, contractName);
  if (!contract) {
    throw new Error(`Contract ${contractGroup}.${contractName} not found`);
  }

  const validatorKey = `${contractGroup}:${contractName}`;
  let validator = validatorCache.get(validatorKey);
  if (!validator) {
    validator = ajv.compile(contract.schema);
    validatorCache.set(validatorKey, validator);
  }

  const valid = validator(payload);
  if (!valid) {
    const error = new Error(
      `Contract validation failed for ${contractGroup}.${contractName}`,
    );
    error.details = validator.errors;
    error.status = 422;
    throw error;
  }

  return true;
}
