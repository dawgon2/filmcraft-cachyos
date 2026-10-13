// SPDX-License-Identifier: MIT
// FilmCraft fork: test for the KIO thumbnail plugin (FORK-PLAN §1a.46).
//
// thumbtest <plugin.so> <scratch dir> [real filmcraft-cli] [real project]
// Loads the plugin like Dolphin's thumbnail worker, swaps in fake CLIs through $FILMCRAFT_CLI and
// checks success and every failure path; with a real CLI and project, renders a real thumbnail.

#include <KIO/ThumbnailCreator>
#include <KPluginFactory>
#include <KPluginMetaData>

#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QGuiApplication>
#include <QImage>
#include <QMimeDatabase>
#include <QUrl>

#include <cstdio>
#include <memory>

static int failures = 0;

static void check(bool ok, const char *what)
{
    std::printf("%s %s\n", ok ? "ok  " : "FAIL", what);
    if (!ok) {
        ++failures;
    }
}

static QString script(const QDir &dir, const QString &name, const QByteArray &body)
{
    const QString path = dir.filePath(name);
    QFile f(path);
    if (f.open(QIODevice::WriteOnly)) {
        f.write("#!/bin/sh\n" + body + "\n");
        f.close();
    }
    f.setPermissions(QFileDevice::ReadOwner | QFileDevice::WriteOwner | QFileDevice::ExeOwner);
    return path;
}

static KIO::ThumbnailResult run(KIO::ThumbnailCreator *c, const QString &file, int size)
{
    const KIO::ThumbnailRequest req(QUrl::fromLocalFile(file), QSize(size, size), QStringLiteral("application/x-filmcraft-project"), 1.0, 0);
    return c->create(req);
}

int main(int argc, char **argv)
{
    QGuiApplication app(argc, argv);
    if (argc < 3) {
        std::fprintf(stderr, "usage: thumbtest <plugin> <scratch dir> [cli] [project]\n");
        return 2;
    }
    const QString pluginPath = QString::fromLocal8Bit(argv[1]);
    QDir dir(QString::fromLocal8Bit(argv[2]));
    dir.removeRecursively();
    QDir().mkpath(dir.path());

    const KPluginMetaData md(pluginPath);
    check(md.isValid(), "plugin metadata loads");
    check(md.mimeTypes().contains(QStringLiteral("application/x-filmcraft-project")), "plugin claims FilmCraft projects");
    auto result = KPluginFactory::instantiatePlugin<KIO::ThumbnailCreator>(md);
    check(bool(result), "plugin instantiates");
    if (!result) {
        return 1;
    }
    std::unique_ptr<KIO::ThumbnailCreator> creator(result.plugin);

    // a project file (contents don't matter to the fake CLIs)
    const QString project = dir.filePath(QStringLiteral("Film.fcproj"));
    QFile pf(project);
    if (pf.open(QIODevice::WriteOnly)) {
        pf.write("{}");
        pf.close();
    }
    // a PNG the fake CLI hands back
    const QString png = dir.filePath(QStringLiteral("src.png"));
    QImage src(160, 90, QImage::Format_RGB32);
    src.fill(Qt::darkCyan);
    src.save(png, "PNG");

    // success: the CLI writes its 4th argument; the size argument is the requested size, bounded
    const QString args = dir.filePath(QStringLiteral("args.txt"));
    const QString good = script(dir, QStringLiteral("good-cli"), "echo \"$@\" > '" + args.toLocal8Bit() + "'\ncp '" + png.toLocal8Bit() + "' \"$4\"");
    qputenv("FILMCRAFT_CLI", good.toLocal8Bit());
    auto r = run(creator.get(), project, 256);
    check(r.isValid() && r.image().size() == QSize(160, 90), "a good CLI gives the picture");
    QFile af(args);
    const QByteArray argLine = af.open(QIODevice::ReadOnly) ? af.readAll() : QByteArray();
    check(argLine.startsWith("thumbnail ") && argLine.contains(" 256 "), "runs `thumbnail <file> <size> <out>`");
    r = run(creator.get(), project, 100000);
    QFile af2(args);
    const QByteArray argLine2 = af2.open(QIODevice::ReadOnly) ? af2.readAll() : QByteArray();
    check(argLine2.contains(" 2048 "), "a huge size is capped at 2048");
    {
        const KIO::ThumbnailRequest hidpi(QUrl::fromLocalFile(project), QSize(128, 128), QStringLiteral("application/x-filmcraft-project"), 2.0, 0);
        r = creator->create(hidpi);
        QFile af3(args);
        const QByteArray argLine3 = af3.open(QIODevice::ReadOnly) ? af3.readAll() : QByteArray();
        check(argLine3.contains(" 256 ") && r.isValid() && r.image().devicePixelRatio() == 2.0, "HiDPI renders device pixels");
    }

    // failures: never a picture, never a crash
    qputenv("FILMCRAFT_CLI", script(dir, QStringLiteral("fail-cli"), "echo broken >&2; exit 1").toLocal8Bit());
    check(!run(creator.get(), project, 128).isValid(), "a failing CLI gives no picture");
    qputenv("FILMCRAFT_CLI", script(dir, QStringLiteral("garbage-cli"), "printf 'not a png' > \"$4\"").toLocal8Bit());
    check(!run(creator.get(), project, 128).isValid(), "a bad PNG gives no picture");
    qputenv("FILMCRAFT_CLI", script(dir, QStringLiteral("silent-cli"), "exit 0").toLocal8Bit());
    check(!run(creator.get(), project, 128).isValid(), "no output file gives no picture");
    qputenv("FILMCRAFT_CLI", dir.filePath(QStringLiteral("no-such-cli")).toLocal8Bit());
    check(!run(creator.get(), project, 128).isValid(), "a missing CLI gives no picture");
    qputenv("FILMCRAFT_CLI", good.toLocal8Bit());
    check(!run(creator.get(), dir.filePath(QStringLiteral("missing.fcproj")), 128).isValid(), "a missing project gives no picture");
    check(!run(creator.get(), dir.path(), 128).isValid(), "a folder gives no picture");

    // optional: the real CLI on a real project
    if (argc >= 5) {
        qputenv("FILMCRAFT_CLI", QByteArray(argv[3]));
        r = run(creator.get(), QString::fromLocal8Bit(argv[4]), 256);
        check(r.isValid() && qMax(r.image().width(), r.image().height()) <= 256 && r.image().width() > 0, "the real CLI renders a project");
        if (r.isValid()) {
            r.image().save(dir.filePath(QStringLiteral("real.png")), "PNG");
        }
    }
    std::printf("%d failure(s)\n", failures);
    return failures == 0 ? 0 : 1;
}
