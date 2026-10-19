// 只同步 Git 提交，不编译、不打包；不删除远端引用，不强推非快进分支或已存在标签。
import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'

assert.equal(process.env.GITHUB_REPOSITORY, 'TanCan1998/Lorekeeper', '禁止同步其他仓库')
const token = process.env.ATOMGIT_ACCESS_TOKEN
assert(token, '缺少 AtomGit 仓库 Secret')
const run = args => execFileSync('git', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim()
try {
  run(['fetch', '--no-tags', 'origin', '+refs/heads/*:refs/remotes/atomgit-source/*', 'refs/tags/*:refs/tags/*'])
  const refs = run(['for-each-ref', '--format=%(objectname) %(refname)', 'refs/remotes/atomgit-source/', 'refs/tags/'])
    .split('\n').filter(Boolean).map(line => {
      const [sha, source] = line.split(' ')
      assert(/^[a-f0-9]{40}$/.test(sha), '同步对象无效')
      const destination = source.replace('refs/remotes/atomgit-source/', 'refs/heads/')
      assert(destination.startsWith('refs/heads/') || destination.startsWith('refs/tags/'))
      return `${sha}:${destination}`
    })
  assert(refs.length > 0, '没有可同步的分支或标签')
  // 凭据只在子进程环境中传递，不拼入远端地址、命令参数或 Git 配置文件。
  const env = {
    ...process.env, GIT_TERMINAL_PROMPT: '0', GCM_INTERACTIVE: 'Never',
    GIT_CONFIG_COUNT: '2',
    GIT_CONFIG_KEY_0: 'http.https://atomgit.com/.extraheader',
    GIT_CONFIG_VALUE_0: 'Authorization: Basic ' + Buffer.from('TanCan1998:' + token).toString('base64'),
    GIT_CONFIG_KEY_1: 'credential.helper', GIT_CONFIG_VALUE_1: '',
  }
  execFileSync('git', ['push', '--atomic', 'https://atomgit.com/TanCan1998/Lorekeeper.git', ...refs], {
    env, stdio: ['ignore', 'pipe', 'pipe'],
  })
  console.log('AtomGit 分支与标签同步完成；未强推、未删除远端内容。')
} catch {
  // Git 的错误可能包含远端认证信息，不能直接打印原始子进程异常。
  console.error('AtomGit 代码同步失败；请检查仓库 Secret、权限或非快进冲突，未执行强推。')
  process.exitCode = 1
}
