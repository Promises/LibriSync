import { useState } from 'react';
import { Check, Copy } from 'lucide-react';
import { Button, type ButtonSize, type ButtonVariant } from './Button';
import { toast } from './Toast';

async function writeClipboard(text: string): Promise<void> {
  if (navigator.clipboard && window.isSecureContext) {
    await navigator.clipboard.writeText(text);
    return;
  }
  // Self-hosted instances are often plain http on a LAN address, where the async
  // clipboard API is unavailable; fall back to a hidden textarea.
  const ta = document.createElement('textarea');
  ta.value = text;
  ta.setAttribute('readonly', '');
  ta.style.position = 'fixed';
  ta.style.opacity = '0';
  document.body.appendChild(ta);
  ta.select();
  const ok = document.execCommand('copy');
  document.body.removeChild(ta);
  if (!ok) throw new Error('Copy is not available in this browser');
}

export function CopyButton({
  text,
  label = 'Copy',
  iconOnly = false,
  size = 'sm',
  variant = 'secondary',
}: {
  text: string;
  label?: string;
  iconOnly?: boolean;
  size?: ButtonSize;
  variant?: ButtonVariant;
}) {
  const [copied, setCopied] = useState(false);
  const onClick = async () => {
    try {
      await writeClipboard(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch (e) {
      toast.error('Could not copy', e instanceof Error ? e.message : undefined);
    }
  };
  return (
    <Button
      size={size}
      variant={variant}
      iconOnly={iconOnly}
      label={copied ? 'Copied' : label}
      icon={copied ? <Check size={14} /> : <Copy size={14} />}
      onClick={onClick}
    >
      {copied ? 'Copied' : label}
    </Button>
  );
}
