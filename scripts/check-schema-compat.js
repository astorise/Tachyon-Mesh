#!/usr/bin/env node
const fs = require('fs');
const path = require('path');

if (process.argv.length !== 4) {
  console.error('usage: check-schema-compat.js <previous.schema.json> <current.schema.json>');
  process.exit(2);
}

const previousPath = process.argv[2];
const currentPath = process.argv[3];
const previous = JSON.parse(fs.readFileSync(previousPath, 'utf8'));
const current = JSON.parse(fs.readFileSync(currentPath, 'utf8'));
const failures = [];

function pointerGet(root, ref) {
  if (!ref.startsWith('#/')) {
    return null;
  }
  return ref
    .slice(2)
    .split('/')
    .reduce((value, part) => {
      if (value === null || typeof value !== 'object') {
        return null;
      }
      const key = part.replace(/~1/g, '/').replace(/~0/g, '~');
      return value[key] ?? null;
    }, root);
}

function resolve(root, schema) {
  if (schema && typeof schema === 'object' && typeof schema.$ref === 'string') {
    return pointerGet(root, schema.$ref) ?? schema;
  }
  return schema;
}

function normalizeTypes(schema) {
  const type = schema?.type;
  if (Array.isArray(type)) {
    return [...type].sort().join('|');
  }
  if (typeof type === 'string') {
    return type;
  }
  if (schema?.const !== undefined) {
    return 'const';
  }
  if (Array.isArray(schema?.enum)) {
    return 'enum';
  }
  if (Array.isArray(schema?.oneOf)) {
    return 'oneOf';
  }
  if (Array.isArray(schema?.anyOf)) {
    return 'anyOf';
  }
  return '';
}

function pathLabel(parts) {
  return parts.length === 0 ? '<root>' : parts.join('.');
}

function compareSchemas(previousRoot, currentRoot, previousSchema, currentSchema, parts = []) {
  previousSchema = resolve(previousRoot, previousSchema);
  currentSchema = resolve(currentRoot, currentSchema);
  if (!previousSchema || !currentSchema) {
    return;
  }

  const previousType = normalizeTypes(previousSchema);
  const currentType = normalizeTypes(currentSchema);
  if (previousType && currentType && previousType !== currentType) {
    failures.push(
      `${pathLabel(parts)} changed type from ${previousType} to ${currentType}`
    );
    return;
  }

  if (Array.isArray(previousSchema.enum) && Array.isArray(currentSchema.enum)) {
    const currentValues = new Set(currentSchema.enum.map((value) => JSON.stringify(value)));
    for (const value of previousSchema.enum) {
      if (!currentValues.has(JSON.stringify(value))) {
        failures.push(`${pathLabel(parts)} removed enum value ${JSON.stringify(value)}`);
      }
    }
  }

  const previousRequired = new Set(previousSchema.required ?? []);
  const currentRequired = new Set(currentSchema.required ?? []);
  for (const field of currentRequired) {
    if (!previousRequired.has(field)) {
      failures.push(`${pathLabel(parts)} added required field ${field}`);
    }
  }

  const previousProperties = previousSchema.properties ?? {};
  const currentProperties = currentSchema.properties ?? {};
  for (const field of Object.keys(previousProperties)) {
    if (!(field in currentProperties)) {
      failures.push(`${pathLabel([...parts, field])} was removed`);
      continue;
    }
    compareSchemas(
      previousRoot,
      currentRoot,
      previousProperties[field],
      currentProperties[field],
      [...parts, field]
    );
  }

  const previousItems = previousSchema.items;
  const currentItems = currentSchema.items;
  if (previousItems && currentItems) {
    compareSchemas(previousRoot, currentRoot, previousItems, currentItems, [...parts, '[]']);
  }

  const previousAdditional = previousSchema.additionalProperties;
  const currentAdditional = currentSchema.additionalProperties;
  if (
    previousAdditional &&
    currentAdditional &&
    typeof previousAdditional === 'object' &&
    typeof currentAdditional === 'object'
  ) {
    compareSchemas(previousRoot, currentRoot, previousAdditional, currentAdditional, [
      ...parts,
      '{}',
    ]);
  }

  for (const key of ['allOf', 'anyOf', 'oneOf']) {
    const previousVariants = previousSchema[key];
    const currentVariants = currentSchema[key];
    if (!Array.isArray(previousVariants) || !Array.isArray(currentVariants)) {
      continue;
    }
    const count = Math.min(previousVariants.length, currentVariants.length);
    for (let index = 0; index < count; index += 1) {
      compareSchemas(previousRoot, currentRoot, previousVariants[index], currentVariants[index], [
        ...parts,
        `${key}[${index}]`,
      ]);
    }
  }
}

compareSchemas(previous, current, previous, current);

if (failures.length > 0) {
  console.error(
    `${path.basename(currentPath)} has ${failures.length} breaking schema change(s):`
  );
  for (const failure of failures) {
    console.error(`- ${failure}`);
  }
  console.error('Add the breaking-manifest label or publish this change as a manifest break.');
  process.exit(1);
}

console.log(`${path.basename(currentPath)} is backward-compatible with ${path.basename(previousPath)}`);
