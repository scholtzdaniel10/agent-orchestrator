export function Star({ size = 12 }: { size?: number }): React.JSX.Element {
  return (
    <svg
      className="star-mark"
      width={size}
      height={size}
      viewBox="0 0 16 16"
      aria-hidden="true"
      focusable="false"
    >
      <path fill="currentColor" d="M8 0 9.1 6.9 16 8 9.1 9.1 8 16 6.9 9.1 0 8 6.9 6.9Z" />
    </svg>
  )
}

export function RayBurst(): React.JSX.Element {
  return <span className="ray-burst" aria-hidden="true" />
}
