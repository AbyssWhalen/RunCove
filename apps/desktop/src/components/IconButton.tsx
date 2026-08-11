import type { ButtonHTMLAttributes, ReactNode } from "react";

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  label: string;
  children: ReactNode;
  tone?: "default" | "danger" | "success";
}

export function IconButton({ label, children, tone = "default", className = "", ...props }: IconButtonProps) {
  return (
    <button
      type="button"
      className={`icon-button icon-button--${tone} ${className}`.trim()}
      aria-label={label}
      title={label}
      data-tooltip={label}
      {...props}
    >
      {children}
    </button>
  );
}
