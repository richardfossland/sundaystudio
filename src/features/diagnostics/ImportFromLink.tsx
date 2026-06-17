import { useState } from "react";
import { AlertTriangle, CheckCircle2, Link2, Loader2 } from "lucide-react";

import { Button } from "@/components/ui/Button";
import { ipc, errorMessage } from "@/lib/ipc";
import { seedProjectFromImportLink } from "@/lib/deeplinkImport";
import { useI18n } from "@/lib/i18n";
import { useSession } from "@/lib/session";

/**
 * "Import from Sunday link" affordance.
 *
 * Accepts a pasted `sundaystudio://import?…` deep link (the same URL SundayRec
 * launches us with), parses it via `deeplink_parse_import`, and seeds a fresh
 * project containing the referenced recording. This makes the Rec → Studio
 * handoff reachable and testable now, before the OS scheme auto-registration
 * (which needs the bundled app + a real open-url event) is wired.
 *
 * `onImported` lets the host refresh its project list after a successful seed.
 */
export function ImportFromLink({ onImported }: { onImported?: () => void }) {
  const t = useI18n((s) => s.t);
  const setHandoff = useSession((s) => s.setHandoff);
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [doneName, setDoneName] = useState<string | null>(null);

  async function handleImport() {
    const trimmed = url.trim();
    if (!trimmed || busy) return;
    setBusy(true);
    setError(null);
    setDoneName(null);
    try {
      const { meta, request } = await seedProjectFromImportLink(ipc, trimmed);
      // Carry the handoff's context/glossary so the editor's show-notes panel
      // can prime Claude (spell speaker names right, name chapters sensibly).
      setHandoff(request.context ?? null, request.glossary ?? []);
      setDoneName(meta.name);
      setUrl("");
      onImported?.();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mt-4 border-t border-[var(--color-border)] pt-4">
      <div className="mb-1 flex items-center gap-2">
        <Link2 size={15} className="text-[var(--color-fg-muted)]" />
        <h3 className="text-ui-sm font-semibold">{t("importLinkTitle")}</h3>
      </div>
      <p className="mb-3 text-ui-xs text-[var(--color-fg-muted)]">
        {t("importLinkDesc")}
      </p>

      <div className="flex items-center gap-2">
        <input
          type="text"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && handleImport()}
          placeholder={t("importLinkPlaceholder")}
          spellCheck={false}
          className="flex-1 rounded-[var(--radius-sm)] border border-[var(--color-border)] bg-[var(--color-bg-surface)] px-3 py-1.5 font-mono text-ui-xs outline-none focus:border-[var(--color-accent)]"
        />
        <Button
          variant="surface"
          size="sm"
          onClick={handleImport}
          disabled={busy || url.trim().length === 0}
        >
          {busy ? (
            <Loader2 size={15} className="animate-spin" />
          ) : (
            <Link2 size={15} />
          )}
          {busy ? t("importLinkImporting") : t("importLinkButton")}
        </Button>
      </div>

      <p className="mt-2 text-[11px] text-[var(--color-fg-muted)]">
        {t("importLinkHint")}
      </p>

      {doneName && (
        <div className="mt-3 flex items-start gap-2 rounded-[var(--radius-md)] bg-[var(--color-bg-surface)] p-2 text-ui-xs text-[var(--color-success)]">
          <CheckCircle2 size={14} className="mt-0.5 shrink-0" />
          <span className="break-all text-[var(--color-fg)]">
            {t("importLinkDone").replace("{name}", doneName)}
          </span>
        </div>
      )}
      {error && (
        <div className="mt-3 flex items-start gap-2 rounded-[var(--radius-md)] bg-[var(--color-bg-surface)] p-2 text-ui-xs text-[var(--color-danger)]">
          <AlertTriangle size={14} className="mt-0.5 shrink-0" />
          <span className="break-all">{error}</span>
        </div>
      )}
    </div>
  );
}
