import { Component, type ReactNode } from 'react'

interface Props {
  children: ReactNode
  className?: string
}

interface State {
  error: Error | null
  nonce: number
}

class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null, nonce: 0 }

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error }
  }

  render(): ReactNode {
    const extra = this.props.className ?? ''
    if (this.state.error !== null) {
      return (
        <div className={['part-error', extra].filter((name) => name !== '').join(' ')} role="alert">
          <p className="part-error-title">This part of the window hit an error.</p>
          <pre className="part-error-message">{this.state.error.message}</pre>
          <button
            type="button"
            className="btn btn-quiet"
            onClick={() => {
              this.setState((current) => ({ error: null, nonce: current.nonce + 1 }))
            }}
          >
            Reload this part
          </button>
        </div>
      )
    }
    return (
      <div
        key={this.state.nonce}
        className={['error-boundary-slot', extra].filter((name) => name !== '').join(' ')}
      >
        {this.props.children}
      </div>
    )
  }
}

export default ErrorBoundary
