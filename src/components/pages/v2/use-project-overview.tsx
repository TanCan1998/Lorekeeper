import { Archive } from 'lucide-react'

import { ipc } from '../../../services/ipc-client'
import { useLayoutStore } from '../../../stores/layout-store'
import { navigate } from '../../../stores/navigation/navigation-intents'
import { useLocaleStore } from '../../../stores/locale-store'
import { useProjectStore } from '../../../stores/project-store'
import { confirmDeleteCurrentProject } from '../../project-delete-action'
import { useShelfOverview } from '../../stage/library/use-shelf-overview'
import { UpdateSection } from '../../updates/UpdateSection'
import WelcomePageV2 from './WelcomePageV2'

export function WriterWelcomePage({ onNewProject }: { onNewProject: () => void }) {
  const { overview, recentProjects, hasCurrentProject, deletableCurrentProject } = useShelfOverview()
  const text = useLocaleStore(state => state.text)
  const legacyImportNotice = useProjectStore(state => state.legacyImportNotice)
  const legacyImportBusy = useProjectStore(state => state.legacyImportBusy)
  const importLegacyProject = useProjectStore(state => state.importLegacyProject)
  return <WelcomePageV2
    overview={overview}
    recentProjects={recentProjects}
    onNewProject={onNewProject}
    onOpenProject={() => {
      void ipc.invoke('dialog:select-folder', 'project-open').then(folder => {
        if (folder) void useProjectStore.getState().openProject(folder)
      })
    }}
    onImportLegacyProject={() => { void importLegacyProject() }}
    legacyImportNotice={legacyImportNotice}
    legacyImportBusy={legacyImportBusy}
    onImportNovel={() => useLayoutStore.getState().openImportNovel()}
    onContinue={hasCurrentProject ? () => navigate({ kind: 'stage' }) : undefined}
    onDeleteCurrentProject={deletableCurrentProject ? () => void confirmDeleteCurrentProject(deletableCurrentProject) : undefined}
    backup={<button type="button" onClick={() => navigate({ kind: 'settings', section: 'backup' })}>
      <Archive size={17} />
      {text('项目备份', 'Project backup')}
    </button>}
    updates={<UpdateSection />}
  />
}
