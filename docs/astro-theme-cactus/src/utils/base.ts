/**
 * Prefix a root-relative path with the site's base URL.
 * Works locally (`/` ) and on GitHub project Pages (`/repo-name/`).
 * Astro injects `import.meta.env.BASE_URL` from `base` in astro.config.ts.
 */
export function withBase(path: string): string {
	const base: string = import.meta.env.BASE_URL ?? "/";
	return `${base}${path.replace(/^\/+/, "")}`;
}
