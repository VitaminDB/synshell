syn-vm guest tools / Гостевые инструменты syn-vm

Run install.cmd as administrator (or double-click it: it asks for elevation).
Запустите install.cmd от администратора (или двойным щелчком — он попросит права).

Installs / ставит:
  - drivers\syn-vm-serial — virtio-serial driver for the syn-vm guest agent port
  - agent\syn-vm-agent.exe — syn-vm guest agent (service "syn-vm-agent")
  - syn-gpu\ — syn-gpu video card tools (only if the machine has the syn-gpu device)

Test-signed drivers: Secure Boot must be off; test signing is enabled automatically
(one reboot, setup then finishes by itself). Log: C:\ProgramData\syn-vm\install.log
