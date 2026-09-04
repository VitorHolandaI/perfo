import QtQuick
import qs.Commons
import qs.Ui

Column {
  id: gpuPage

  property var devices: []
  property color foreground: Color.foreground
  property string fontFamily: Style.font.family

  spacing: Style.space(8)

  Text { text: "GRAPHICS"; color: gpuPage.foreground; opacity: 0.65; font.family: gpuPage.fontFamily; font.pixelSize: Style.font.caption }
  Text { text: gpuPage.devices.length > 0 ? gpuPage.devices.length + " DETECTED GPU" + (gpuPage.devices.length === 1 ? "" : "S") : "NO READABLE GPUS"; color: gpuPage.foreground; font.family: gpuPage.fontFamily; font.pixelSize: Style.font.subtitle; font.bold: true }

  Repeater {
    model: gpuPage.devices
    delegate: Column {
      width: gpuPage.width
      spacing: Style.space(3)

      Row {
        width: parent.width
        Text { width: parent.width - Style.space(90); text: modelData.name + " [" + modelData.vendor + "]"; color: gpuPage.foreground; font.family: gpuPage.fontFamily; font.pixelSize: Style.font.bodySmall; elide: Text.ElideRight }
        Text { width: Style.space(90); text: modelData.usage_percent === null ? "--" : Number(modelData.usage_percent).toFixed(0) + "%"; color: gpuPage.foreground; font.family: gpuPage.fontFamily; font.pixelSize: Style.font.bodySmall; horizontalAlignment: Text.AlignRight }
      }
      Rectangle { width: parent.width; height: Style.space(10); color: gpuPage.foreground; opacity: 0.15; Rectangle { width: parent.width * gpuPage.percent(modelData.usage_percent) / 100; height: parent.height; color: Color.accent; opacity: 1 } }
      Text { text: "MEM " + (modelData.memory_used_bytes === null || modelData.memory_total_bytes === null ? "--" : gpuPage.formatBytes(modelData.memory_used_bytes) + " / " + gpuPage.formatBytes(modelData.memory_total_bytes)); color: gpuPage.foreground; opacity: 0.7; font.family: gpuPage.fontFamily; font.pixelSize: Style.font.bodySmall }
    }
  }

  function percent(value) {
    var number = Number(value)
    return isFinite(number) ? Math.max(0, Math.min(100, number)) : 0
  }

  function formatBytes(bytes) {
    if (bytes === null || bytes === undefined) return "--"
    var value = Number(bytes)
    if (!isFinite(value)) return "--"
    if (value >= 1073741824) return (value / 1073741824).toFixed(1) + "G"
    if (value >= 1048576) return (value / 1048576).toFixed(0) + "M"
    if (value >= 1024) return (value / 1024).toFixed(0) + "K"
    return Math.round(value) + "B"
  }
}
