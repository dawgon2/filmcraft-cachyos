// SPDX-License-Identifier: MIT
// FilmCraft fork: Dolphin / KIO thumbnails of FilmCraft projects (FORK-PLAN §1a.46).
//
// Original code. The picture is drawn by `filmcraft-cli thumbnail <file> <size> <out.png>` (Rust);
// this plugin only runs it with a timeout and hands the PNG to KIO. A missing CLI, a broken
// project, a timeout or a bad PNG all end in ThumbnailResult::fail(): Dolphin then shows the
// MIME icon, nothing crashes.

#include <KIO/ThumbnailCreator>
#include <KPluginFactory>

#include <QDir>
#include <QFileInfo>
#include <QImage>
#include <QProcess>
#include <QStandardPaths>
#include <QTemporaryDir>

#ifndef FILMCRAFT_CLI
#define FILMCRAFT_CLI "filmcraft-cli"
#endif
#ifndef FILMCRAFT_THUMB_TIMEOUT_S
#define FILMCRAFT_THUMB_TIMEOUT_S 20
#endif

namespace
{
// The CLI to run: $FILMCRAFT_CLI, else the configured path, else a name looked up in PATH.
QString cliPath()
{
    const QString fromEnv = qEnvironmentVariable("FILMCRAFT_CLI");
    const QString configured = fromEnv.isEmpty() ? QStringLiteral(FILMCRAFT_CLI) : fromEnv;
    if (QFileInfo(configured).isAbsolute()) {
        return QFileInfo(configured).isExecutable() ? configured : QString();
    }
    return QStandardPaths::findExecutable(configured);
}
}

class FilmCraftThumbnailer : public KIO::ThumbnailCreator
{
    Q_OBJECT
public:
    FilmCraftThumbnailer(QObject *parent, const QVariantList &args)
        : KIO::ThumbnailCreator(parent, args)
    {
    }

    KIO::ThumbnailResult create(const KIO::ThumbnailRequest &request) override
    {
        const QString input = request.url().toLocalFile();
        if (input.isEmpty() || !QFileInfo(input).isFile()) {
            return KIO::ThumbnailResult::fail();
        }
        const QString cli = cliPath();
        if (cli.isEmpty()) {
            return KIO::ThumbnailResult::fail();
        }
        // HiDPI: render device pixels and tag the image with the ratio
        const qreal dpr = qBound(1.0, request.devicePixelRatio(), 4.0);
        const QSize target = request.targetSize() * dpr;
        const int size = qBound(16, qMax(target.width(), target.height()), 2048);
        QTemporaryDir dir;
        if (!dir.isValid()) {
            return KIO::ThumbnailResult::fail();
        }
        const QString out = dir.filePath(QStringLiteral("thumb.png"));

        QProcess p;
        p.setProcessChannelMode(QProcess::MergedChannels);
        p.setStandardInputFile(QProcess::nullDevice());
        p.start(cli, {QStringLiteral("thumbnail"), input, QString::number(size), out});
        if (!p.waitForStarted(5000)) {
            return KIO::ThumbnailResult::fail();
        }
        if (!p.waitForFinished(FILMCRAFT_THUMB_TIMEOUT_S * 1000)) {
            p.kill();
            p.waitForFinished(2000);
            return KIO::ThumbnailResult::fail();
        }
        if (p.exitStatus() != QProcess::NormalExit || p.exitCode() != 0) {
            return KIO::ThumbnailResult::fail();
        }
        QImage image;
        if (!image.load(out, "PNG") || image.isNull()) {
            return KIO::ThumbnailResult::fail();
        }
        image.setDevicePixelRatio(dpr);
        return KIO::ThumbnailResult::pass(image);
    }
};

K_PLUGIN_CLASS_WITH_JSON(FilmCraftThumbnailer, "filmcraftthumbnail.json")

#include "filmcraftthumbnail.moc"
