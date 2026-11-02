// The same id rules as src/preprint.rs, so the home page can route straight
// to /abs/<id>. The server re-checks everything it is sent.

const NEW_STYLE = /^(\d{4})\.(\d{4,5})(?:v\d+)?$/
const OLD_STYLE = /^([a-z][a-z-]*(?:\.[A-Za-z]{2})?)\/(\d{7})(?:v\d+)?$/
// 10.1101/2024.06.28.601293, optionally with a version or .full/.pdf/.source suffix.
const DOI = /^(10\.1101\/[\d.]+)(?:v\d+)?(?:\.(?:full|pdf|source))?$/

export function parseId(input: string): string | null {
  let s = input.trim().replace(/^https?:\/\//i, '')
  s = s.replace(/^(www\.|export\.)?arxiv\.org\/(abs|pdf|html|format|src|e-print)\//i, '')
  s = s.replace(/^aero\.oops\.wtf\/paper-bro\/(abs|pdf|html)\//i, '')
  s = s.replace(/^arxiv:/i, '')
  // bioRxiv / medRxiv content pages: /content/10.1101/…v1.full
  s = s.replace(/^(www\.)?(biorxiv|medrxiv)\.org\/content\//i, '')
  s = s.replace(/^doi\.org\//i, '')
  s = s.split(/[?#]/)[0].replace(/\/+$/, '').replace(/\.pdf$/i, '')
  const m = NEW_STYLE.exec(s)
  if (m) {
    const fiveDigits = m[1] >= '1501'
    return fiveDigits === (m[2].length === 5) ? `${m[1]}.${m[2]}` : null
  }
  const o = OLD_STYLE.exec(s)
  if (o) {
    const [archive, cls] = o[1].split('.')
    return `${archive.toLowerCase()}${cls ? '.' + cls.toUpperCase() : ''}/${o[2]}`
  }
  const d = DOI.exec(s)
  if (d) return d[1]
  return null
}

/** "arxiv" | "biorxiv" | "medrxiv" → "arXiv" | "bioRxiv" | "medRxiv" */
export function sourceLabel(source: string | null | undefined): string {
  switch ((source ?? '').toLowerCase()) {
    case 'biorxiv':
      return 'bioRxiv'
    case 'medrxiv':
      return 'medRxiv'
    default:
      return 'arXiv'
  }
}
