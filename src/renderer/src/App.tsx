import LeadChat from './LeadChat'
import Workers from './Workers'
import UsageMeter from './UsageMeter'

function App(): React.JSX.Element {
  return (
    <div className="app">
      <section className="panel panel-lead" aria-labelledby="lead-heading">
        <h2 id="lead-heading">Lead</h2>
        <LeadChat />
      </section>
      <section className="panel panel-workers" aria-labelledby="workers-heading">
        <h2 id="workers-heading">Workers</h2>
        <Workers />
      </section>
      <section className="panel panel-usage" aria-labelledby="usage-heading">
        <h2 id="usage-heading">Usage</h2>
        <UsageMeter />
      </section>
    </div>
  )
}

export default App
