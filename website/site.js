const copyButton = document.querySelector('#copy-command');
if (copyButton && navigator.clipboard?.writeText) {
  copyButton.hidden = false;
  copyButton.addEventListener('click', async () => {
    const result = document.querySelector('#copy-result');
    try {
      await navigator.clipboard.writeText(document.querySelector('#build-command').textContent);
      copyButton.textContent = 'Copied!';
      result.textContent = 'Build commands copied to clipboard.';
    } catch {
      copyButton.textContent = 'Select commands below';
      result.textContent = 'Clipboard unavailable. Select and copy the commands below.';
    }
  });
}
