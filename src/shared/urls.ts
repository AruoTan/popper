export type UrlValidationResult = { valid: true } | { valid: false; reason: string };

const HTTP_PROTOCOLS = new Set(["http:", "https:"]);

function parseHttpUrl(value: string): URL | null {
  try {
    const url = new URL(value);
    if (!HTTP_PROTOCOLS.has(url.protocol) || url.username || url.password) {
      return null;
    }
    return url;
  } catch {
    return null;
  }
}

export function isSafeExternalUrl(value: string): boolean {
  return parseHttpUrl(value) !== null;
}

export function validateOpenAiBaseUrl(value: string): UrlValidationResult {
  if (typeof value !== "string" || value.length === 0) {
    return { valid: false, reason: "API 地址不能为空" };
  }
  if (value.length > 2_048) {
    return { valid: false, reason: "API 地址过长" };
  }

  const url = parseHttpUrl(value);
  if (!url || url.search || url.hash) {
    return { valid: false, reason: "API 地址必须是无查询参数的 HTTP 或 HTTPS 地址" };
  }
  return { valid: true };
}

export function normalizeOpenAiBaseUrl(value: string): string {
  const validation = validateOpenAiBaseUrl(value);
  if (!validation.valid) {
    throw new TypeError(validation.reason);
  }
  return value.replace(/\/+$/, "");
}

export function buildOpenAiEndpoint(
  baseUrl: string,
  endpoint: "models" | "chat/completions",
): string {
  return `${normalizeOpenAiBaseUrl(baseUrl)}/${endpoint}`;
}
