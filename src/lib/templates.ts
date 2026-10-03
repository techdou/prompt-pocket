export interface TemplateField { name: string; defaultValue: string }
const tokens = () => /\\?\{\{\s*([^{}|\r\n]+?)(?:\|([^{}]*))?\s*\}\}/g;
export function templateFields(body: string): TemplateField[] {
  const fields = new Map<string, TemplateField>();
  for (const match of body.matchAll(tokens())) {
    if (match[0].startsWith("\\")) continue;
    const name = match[1].trim();
    if (name && !fields.has(name)) fields.set(name, { name, defaultValue: (match[2] ?? "").trim() });
  }
  return [...fields.values()];
}
export function renderTemplate(body: string, values: Record<string, string>): string {
  const defaults = new Map(templateFields(body).map((field) => [field.name, field.defaultValue]));
  return body.replace(tokens(), (token: string, name: string) => token.startsWith("\\") ? token.slice(1) :
    (Object.hasOwn(values, name.trim()) ? values[name.trim()] : defaults.get(name.trim()) ?? ""));
}
export function missingTemplateFields(body: string, values: Record<string, string>): string[] {
  return templateFields(body).filter((field) => !(Object.hasOwn(values, field.name) ? values[field.name] : field.defaultValue).trim()).map((field) => field.name);
}
