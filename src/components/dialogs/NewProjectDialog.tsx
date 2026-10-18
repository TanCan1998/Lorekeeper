import { FolderOpen, Sparkles } from 'lucide-react'
import {
  Dialog, DialogContent, DialogHeader, DialogFooter, DialogTitle, DialogDescription,
} from '../ui/Dialog'
import { Button } from '../ui/Button'
import { Input } from '../ui/Input'
import { Label } from '../ui/Label'
import { useLocaleStore } from '../../stores/locale-store'
import { useNewProjectForm } from './use-new-project-form'

interface NewProjectDialogProps {
  open: boolean
  onClose: () => void
}

/** 新建项目对话框 */
export default function NewProjectDialog({ open, onClose }: NewProjectDialogProps) {
  const { text } = useLocaleStore()
  const { name, setName, path, selectFolder: handleSelectFolder, creating, canCreate, create: handleCreate } = useNewProjectForm(open, onClose)

  return (
    <Dialog open={open} onOpenChange={(v) => !v && onClose()}>
      <DialogContent className="max-w-[420px]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Sparkles size={18} className="text-[var(--color-accent)]" />
            {text('新建小说项目', 'New novel project')}
          </DialogTitle>
          <DialogDescription>{text('填写作品名称和保存位置，其余配置在项目内完成', 'Enter a title and save location; configure the rest inside the project.')}</DialogDescription>
        </DialogHeader>

        {/* 表单 */}
        <div className="px-5 py-4 space-y-4">
          {/* 项目名称 */}
          <div>
            <Label>{text('作品名称', 'Title')}</Label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={text('如：斗破苍穹', 'e.g. The Glass Observatory')}
              autoFocus
              onKeyDown={(e) => e.key === 'Enter' && handleCreate()}
            />
          </div>

          {/* 保存路径 */}
          <div>
            <Label>{text('保存位置', 'Save location')}</Label>
            <div className="flex gap-2">
              <Input
                value={path?.displayName ?? ''}
                readOnly
                placeholder={text('选择项目保存目录', 'Choose a project folder')}
                className="flex-1"
              />
              <Button variant="outline" onClick={handleSelectFolder}>
                <FolderOpen size={14} />
                {text('选择', 'Choose')}
              </Button>
            </div>
          </div>
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>{text('取消', 'Cancel')}</Button>
          <Button
            onClick={handleCreate}
            disabled={!canCreate}
          >
            <Sparkles size={14} />
            {creating ? text('创建中...', 'Creating...') : text('创建项目', 'Create project')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
