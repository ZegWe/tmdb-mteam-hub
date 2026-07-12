import fs from "node:fs";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "../..");
const openapi = JSON.parse(fs.readFileSync(path.join(root, "src/http/openapi.json"), "utf8"));
const modules = [
  ["health.rs", ""],
  ["auth.rs", "/api"],
  ["config.rs", "/api"],
  ["douban.rs", "/api"],
  ["media.rs", "/api"],
  ["mteam.rs", "/api"],
  ["operation_logs.rs", "/api"],
  ["qb.rs", "/api"],
  ["subscription_queries.rs", "/api"],
  ["subscriptions.rs", "/api"],
];
const httpMethods = new Set(["get", "post", "put", "patch", "delete"]);

function fail(message) {
  process.stderr.write(`OpenAPI parity failed: ${message}\n`);
  process.exitCode = 1;
}

function balancedRouteCalls(source) {
  const calls = [];
  let offset = 0;
  while ((offset = source.indexOf(".route(", offset)) !== -1) {
    const start = offset + ".route(".length;
    let depth = 1;
    let quote = null;
    let escaped = false;
    let cursor = start;
    for (; cursor < source.length && depth > 0; cursor += 1) {
      const char = source[cursor];
      if (quote) {
        if (escaped) escaped = false;
        else if (char === "\\") escaped = true;
        else if (char === quote) quote = null;
        continue;
      }
      if (char === '"' || char === "'") quote = char;
      else if (char === "(") depth += 1;
      else if (char === ")") depth -= 1;
    }
    if (depth !== 0) throw new Error("unbalanced .route(...) expression");
    calls.push(source.slice(start, cursor - 1));
    offset = cursor;
  }
  return calls;
}

function sourceRoutes() {
  const routes = [];
  for (const [file, prefix] of modules) {
    const source = fs.readFileSync(path.join(root, "src/http", file), "utf8");
    for (const call of balancedRouteCalls(source)) {
      const pathMatch = call.match(/^\s*"([^"]+)"\s*,/);
      if (!pathMatch) continue;
      const methods = [...call.matchAll(/(?:^|[.\s])(get|post|put|patch|delete)\s*\(/g)].map(
        (match) => match[1],
      );
      for (const method of new Set(methods))
        routes.push(`${method.toUpperCase()} ${prefix}${pathMatch[1]}`);
    }
  }
  return [...new Set(routes)].sort();
}

function documentedRoutes() {
  const routes = [];
  for (const [routePath, pathItem] of Object.entries(openapi.paths ?? {})) {
    for (const method of Object.keys(pathItem)) {
      if (httpMethods.has(method)) routes.push(`${method.toUpperCase()} ${routePath}`);
    }
  }
  return routes.sort();
}

const actual = sourceRoutes();
const documented = documentedRoutes();
const missing = actual.filter((route) => !documented.includes(route));
const stale = documented.filter((route) => !actual.includes(route));
if (missing.length) fail(`production routes missing from OpenAPI: ${missing.join(", ")}`);
if (stale.length) fail(`OpenAPI routes missing from production Router: ${stale.join(", ")}`);

const publicRoutes = new Set([
  "GET /healthz",
  "GET /readyz",
  "GET /api/auth/status",
  "POST /api/auth/login",
  "POST /api/auth/logout",
]);
const inheritedSecurity = JSON.stringify(openapi.security ?? []);
if (inheritedSecurity !== JSON.stringify([{ ManagementSession: [] }])) {
  fail("top-level security must require ManagementSession");
}

for (const name of [
  "ManagementUnauthorized",
  "MethodNotAllowed",
  "InternalError",
  "ServiceUnavailable",
]) {
  const response = openapi.components?.responses?.[name];
  if (
    response?.content?.["application/json"]?.schema?.$ref !== "#/components/schemas/ApiErrorDto"
  ) {
    fail(`${name} must use the closed ApiErrorDto contract`);
  }
}
for (const route of documented) {
  const [method, routePath] = route.split(" ");
  const operation = openapi.paths[routePath][method.toLowerCase()];
  if (publicRoutes.has(route)) {
    if (JSON.stringify(operation.security ?? null) !== "[]") {
      fail(`${route} must explicitly opt out of management authentication`);
    }
  } else if (operation.security && operation.security.length === 0) {
    fail(`${route} unexpectedly opts out of management authentication`);
  }
  const expectedResponses = {
    405: "MethodNotAllowed",
    500: "InternalError",
    503: "ServiceUnavailable",
  };
  if (!publicRoutes.has(route)) expectedResponses[401] = "ManagementUnauthorized";
  for (const [status, responseName] of Object.entries(expectedResponses)) {
    const expectedRef = `#/components/responses/${responseName}`;
    if (operation.responses?.[status]?.$ref !== expectedRef) {
      fail(`${route} response ${status} must reference ${expectedRef}`);
    }
  }
}

const closedSchemas = Object.entries(openapi.components?.schemas ?? {}).filter(
  ([, schema]) => schema?.type === "object",
);
for (const [name, schema] of closedSchemas) {
  if (schema.additionalProperties !== false) fail(`${name} must set additionalProperties=false`);
}

function inspectRefs(value) {
  if (!value || typeof value !== "object") return;
  if (typeof value.$ref === "string" && value.$ref.startsWith("#/components/schemas/")) {
    const name = value.$ref.slice("#/components/schemas/".length);
    if (!openapi.components?.schemas?.[name]) fail(`unresolved schema reference ${value.$ref}`);
  }
  for (const child of Object.values(value)) inspectRefs(child);
}
inspectRefs(openapi);

if (!process.exitCode) {
  process.stdout.write(
    `OpenAPI parity verified: ${actual.length} production methods, management security, all object schemas closed\n`,
  );
}
