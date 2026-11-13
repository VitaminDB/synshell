syn-vm guest tools / Гостевые инструменты syn-vm

One disk for all guests, one folder per system.
Один диск для всех гостей, по каталогу на систему.

  <system>\agent\          syn-vm guest agent of that system:
                           win10\agent, win7\agent: syn-vm-agent.exe install (cmd as administrator)
                           linux\agent: sudo ./syn-vm-agent install

  install.cmd              Windows 10/11: everything below in one go (asks for elevation):
  win10\syn-vm-serial\     virtio-serial driver for the agent port
  win10\syn-gpu\           syn-gpu video card tools (only if the machine has syn-gpu)

  win7\                    Windows 7: syn-gpu driver (win7\install.cmd)
  winxp\                   Windows XP: syn-gpu driver
  win9x\                   Windows 95/98/ME: syn-gpu driver, Glide, SYNSETUP.EXE
  dos\                     DOS: SYNGPU, SYNMOUSE, Glide (also on the "DOS tools" disk)
  linux\                   Linux: syn-gpu tools

Test-signed drivers (Windows 7-11): Secure Boot must be off; test signing is enabled
automatically (one reboot, setup then finishes by itself). Log: C:\ProgramData\syn-vm\install.log
Драйверы с тестовой подписью: Secure Boot выключен; тестовая подпись включается сама
(одна перезагрузка, затем установка заканчивается сама).
