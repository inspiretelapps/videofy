import type { ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

export const DDD_HOME = "https://www.doesthedogdie.com";
export const DDD_API = "https://www.doesthedogdie.com/api";
export const DDD_TERMS = "https://www.doesthedogdie.com/api/terms";
export const DDD_SOURCE_KEY = "does-the-dog-die";

export function eventUsesDdd(event: {
  sourceKey: string;
  evidence: Array<{ source: string }>;
}): boolean {
  if (event.sourceKey.split("|").includes(DDD_SOURCE_KEY)) return true;
  return event.evidence.some((item) => isDddSource(item.source));
}

export function isDddSource(source: string): boolean {
  return /doesthedogdie|does the dog die/i.test(source);
}

export function openDddPage(url: string = DDD_HOME): void {
  void openUrl(url);
}

export function DddLink({
  href,
  children,
}: {
  href: string;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      className="text-dust underline decoration-seam underline-offset-2 hover:text-glow"
      onClick={(event) => {
        event.stopPropagation();
        openDddPage(href);
      }}
    >
      {children}
    </button>
  );
}

export function DddAttribution({ compact = false }: { compact?: boolean }) {
  return (
    <p
      className={
        compact
          ? "text-[10px] text-faint"
          : "text-[11px] leading-snug text-faint"
      }
    >
      Powered by <DddLink href={DDD_HOME}>DoesTheDogDie.com</DddLink>
    </p>
  );
}
