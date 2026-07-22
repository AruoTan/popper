import { DEFAULT_SEARCH_TEMPLATE, TEXT_PLACEHOLDER } from './constants'

export type UrlValidationResult =
  | { valid: true }
  | { valid: false; reason: string }

const HTTP_PROTOCOLS = new Set(['http:', 'https:'])

function parseHttpUrl(value: string): URL | null {
  try {
    const url = new URL(value)
    if (!HTTP_PROTOCOLS.has(url.protocol) || url.username || url.password) {
      return null
    }
    return url
  } catch {
    return null
  }
}

export function validateSearchTemplate(template: string): UrlValidationResult {
  if (typeof template !== 'string' || template.length === 0) {
    return { valid: false, reason: '搜索地址不能为空' }
  }
  if (template.length > 2_048) {
    return { valid: false, reason: '搜索地址过长' }
  }
  if (template.split(TEXT_PLACEHOLDER).length - 1 !== 1) {
    return { valid: false, reason: `搜索地址必须且只能包含一个 ${TEXT_PLACEHOLDER}` }
  }

  const candidate = template.replaceAll(TEXT_PLACEHOLDER, 'textlens-validation')
  if (!parseHttpUrl(candidate)) {
    return { valid: false, reason: '搜索地址必须是有效的 HTTP 或 HTTPS 地址' }
  }
  return { valid: true }
}

export function isValidSearchTemplate(template: string): boolean {
  return validateSearchTemplate(template).valid
}

export function buildSearchUrl(template: string, text: string): string {
  const validation = validateSearchTemplate(template)
  if (!validation.valid) {
    throw new TypeError(validation.reason)
  }

  const built = template.replaceAll(TEXT_PLACEHOLDER, encodeURIComponent(text))
  const url = parseHttpUrl(built)
  if (!url) {
    throw new TypeError('生成的搜索地址无效')
  }
  return url.toString()
}

const IPV4_WITH_SUFFIX = /^(?<address>(?:\d{1,3}\.){3}\d{1,3})(?::\d{1,5})?(?:[/?#].*)?$/u
const BRACKETED_IPV6 = /^\[[0-9a-f:.]+\](?::\d{1,5})?(?:[/?#].*)?$/iu
const BARE_IPV6 = /^[0-9a-f]+(?::[0-9a-f]*){2,}$/iu

function isValidIpv4(address: string): boolean {
  const octets = address.split('.')
  return (
    octets.length === 4 &&
    octets.every(
      (octet) =>
        /^(?:0|[1-9]\d{0,2})$/u.test(octet) && Number(octet) >= 0 && Number(octet) <= 255
    )
  )
}

function inferredHttpUrl(value: string): URL | null {
  const inferred = parseHttpUrl(`https://${value}`)
  if (!inferred) return null

  const ipv4 = IPV4_WITH_SUFFIX.exec(value)
  if (ipv4?.groups?.address) {
    return isValidIpv4(ipv4.groups.address) ? inferred : null
  }

  if (BRACKETED_IPV6.test(value)) return inferred

  // URL.hostname is already converted to ASCII/punycode by the URL parser.
  // Requiring a dot avoids treating an ordinary single word as a web address;
  // localhost remains useful when it includes an explicit development port.
  const hostname = inferred.hostname.toLowerCase()
  const isDomain =
    hostname.includes('.') &&
    hostname
      .split('.')
      .every(
        (label) =>
          label.length >= 1 &&
          label.length <= 63 &&
          /^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/u.test(label)
      ) &&
    !/^\d+(?:\.\d+){3}$/u.test(hostname)
  const isLocalhostWithPort = hostname === 'localhost' && inferred.port.length > 0
  return isDomain || isLocalhostWithPort ? inferred : null
}

/** Opens an explicit URL/domain/IP directly; ordinary text becomes a Google search. */
export function resolveSearchTarget(
  text: string,
  searchTemplate = DEFAULT_SEARCH_TEMPLATE
): string {
  const value = text.trim()
  if (value && value.length <= 2_048 && !/\s/u.test(value)) {
    const explicit = parseHttpUrl(value)
    if (explicit) return explicit.toString()

    const inferred = inferredHttpUrl(value)
    if (inferred) return inferred.toString()

    // A bare IPv6 literal needs brackets before it can be represented as an
    // HTTP URL. Let the standards-based URL parser perform the final validity
    // check so malformed compression and oversized groups are rejected.
    if (BARE_IPV6.test(value)) {
      const ipv6 = parseHttpUrl(`https://[${value}]`)
      if (ipv6) return ipv6.toString()
    }
  }
  return buildSearchUrl(searchTemplate, value)
}

export function isSafeExternalUrl(value: string): boolean {
  return parseHttpUrl(value) !== null
}

export function validateOpenAiBaseUrl(value: string): UrlValidationResult {
  if (typeof value !== 'string' || value.length === 0) {
    return { valid: false, reason: 'API 地址不能为空' }
  }
  if (value.length > 2_048) {
    return { valid: false, reason: 'API 地址过长' }
  }

  const url = parseHttpUrl(value)
  if (!url || url.search || url.hash) {
    return { valid: false, reason: 'API 地址必须是无查询参数的 HTTP 或 HTTPS 地址' }
  }
  return { valid: true }
}

export function normalizeOpenAiBaseUrl(value: string): string {
  const validation = validateOpenAiBaseUrl(value)
  if (!validation.valid) {
    throw new TypeError(validation.reason)
  }
  return value.replace(/\/+$/, '')
}

export function buildOpenAiEndpoint(baseUrl: string, endpoint: 'models' | 'chat/completions'): string {
  return `${normalizeOpenAiBaseUrl(baseUrl)}/${endpoint}`
}
