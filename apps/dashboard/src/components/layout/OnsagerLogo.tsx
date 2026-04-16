/**
 * Onsager Logo Component
 *
 * Usage:
 *   import { OnsagerLogo } from './OnsagerLogo'
 *
 *   <OnsagerLogo />                          // 32px, inherits text color
 *   <OnsagerLogo size={64} />                // 64px
 *   <OnsagerLogo className="text-white" />   // Tailwind color control
 *   <OnsagerLogo style={{ color: '#1E40AF' }} />  // inline color
 *
 * The logo uses currentColor — it automatically matches the parent's
 * text color, so dark/light theme works with zero extra code.
 */
export function OnsagerLogo({ size = 32, className = '', style = {}, ...props }: {
  size?: number
  className?: string
  style?: React.CSSProperties
  [key: string]: unknown
}) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 32 32"
      width={size}
      height={size}
      className={className}
      style={style}
      role="img"
      aria-label="Onsager"
      {...props}
    >
      <title>Onsager</title>
      <g fill="currentColor">
        <rect x="2" y="2" width="7" height="7" />
        <rect x="23" y="2" width="7" height="7" />
        <rect x="2" y="23" width="7" height="7" />
        <rect x="23" y="23" width="7" height="7" />
        <rect x="11" y="11" width="10" height="10" />
      </g>
    </svg>
  );
}

export default OnsagerLogo;
