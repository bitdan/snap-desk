const section = document.querySelector('#download')
for (const command of section.querySelectorAll('.install-command')) {
  const button = command.querySelector('button')
  const code = command.querySelector('code')
  const status = command.querySelector('[role=status]')
  let timer
  button.addEventListener('click', async () => {
    clearTimeout(timer)
    status.textContent = ''
    try {
      await navigator.clipboard.writeText(code.textContent)
      status.textContent = section.dataset.copied
      timer = setTimeout(() => { status.textContent = '' }, 2500)
    } catch {
      const selection = window.getSelection()
      const range = document.createRange()
      range.selectNodeContents(code)
      selection.removeAllRanges()
      selection.addRange(range)
      status.textContent = section.dataset.copyFailed
    }
  })
  button.hidden = false
}
