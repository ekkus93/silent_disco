import { Button } from "../../components/Button";
import { Detail as SharedDetail } from "../../components/Detail";
import { ErrorAlert as SharedErrorAlert } from "../../components/ErrorAlert";
import type { DesktopErrorDto } from "../../core/generated/desktop-bindings";

export { StatusCard } from "../../components/StatusCard";

/// This screen's `Detail` broke long values on any character; the shared one
/// defaults to word boundaries, so the behaviour is requested explicitly here
/// rather than forked into a second implementation.
export function Detail({ label, value }: { label: string; value: string }) {
  return <SharedDetail label={label} value={value} wrap="all" />;
}

export function CopyButton({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <Button onClick={onClick} className="px-3">
      {label}
    </Button>
  );
}

export function ErrorAlert({ error }: { error: DesktopErrorDto }) {
  return (
    <div className="mt-4">
      <SharedErrorAlert error={error} />
    </div>
  );
}
