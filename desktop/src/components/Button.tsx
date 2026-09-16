import type { ButtonHTMLAttributes, ReactNode } from "react";

export type ButtonVariant = "primary" | "secondary" | "danger";

const VARIANT_CLASSES: Record<ButtonVariant, string> = {
  primary:
    "border border-cyan-300/50 bg-cyan-500/10 text-cyan-100 hover:border-cyan-200 disabled:border-slate-700 disabled:bg-transparent disabled:text-slate-500",
  secondary:
    "border border-slate-600 text-slate-100 hover:border-slate-400 disabled:border-slate-800 disabled:text-slate-500",
  danger:
    "border border-red-400/50 bg-red-950/30 text-red-100 hover:border-red-300 disabled:border-slate-800 disabled:text-slate-500",
};

/// Replaces the secondary-button utility string that was duplicated verbatim
/// across screens. Disabled styling is part of the variant so a disabled
/// control never reads as merely low-contrast.
export function Button({
  variant = "secondary",
  children,
  className = "",
  type = "button",
  ...rest
}: {
  variant?: ButtonVariant;
  children: ReactNode;
} & ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type={type === "submit" ? "submit" : "button"}
      className={`rounded-lg px-4 py-2 text-sm font-semibold disabled:cursor-not-allowed ${VARIANT_CLASSES[variant]} ${className}`}
      {...rest}
    >
      {children}
    </button>
  );
}
