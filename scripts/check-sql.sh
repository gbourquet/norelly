#!/usr/bin/env bash
# Garde contre l'injection SQL (voir docs/REVIEWS.md, revue de la phase 4) :
# aucune donnée reçue (message, serveur, interface) ne doit entrer dans le
# texte d'une requête ; les valeurs passent toujours par des paramètres
# liés (?1, params![]).
#
# Règles vérifiées :
#   1. seul le crate norelly-sqlite dépend de rusqlite ;
#   2. dans son code de production (hors tests) :
#      - pas d'execute_batch (texte SQL multiple, réservé aux migrations
#        embarquées et aux tests) ;
#      - tout format!/concat! qui produit du SQL est précédé, dans les trois
#        lignes, d'un commentaire « sql-const » attestant que seules des
#        constantes du code y sont interpolées ;
#      - une interpolation dans un tel format! ne peut être qu'une
#        constante en MAJUSCULES ou un paramètre déclaré &'static str ;
#      - pas de push_str ni de concaténation (+) sur du texte SQL.
#
# Ce script fait partie de scripts/check.sh : il est interdit de le
# ramollir pour faire passer une phase.
set -euo pipefail
cd "$(dirname "$0")/.."

status=0

# 1. rusqlite confiné au crate de stockage.
for manifest in $(git ls-files '*Cargo.toml'); do
  case "$manifest" in
    Cargo.toml|crates/adapters/sqlite/Cargo.toml) continue ;;
  esac
  if grep -Eq '^\s*rusqlite' "$manifest"; then
    echo "ERREUR : $manifest dépend de rusqlite (réservé à crates/adapters/sqlite)" >&2
    status=1
  fi
done

# 2. Construction des requêtes dans le code de production.
files=$(git ls-files 'crates/adapters/sqlite/src/*.rs' 'crates/adapters/sqlite/src/**/*.rs' \
  | grep -v '/tests\.rs$' | sort -u)

perl -e '
  use strict; use warnings;
  my $status = 0;
  my $sql = qr/\b(SELECT|INSERT|UPDATE|DELETE|REPLACE|PRAGMA|CREATE|DROP|ALTER|ATTACH|WHERE|VALUES)\b/;
  for my $file (@ARGV) {
    open(my $fh, "<", $file) or die "$file : $!";
    my @lines = <$fh>;
    close $fh;
    # Code de production seulement : on s arrête au module de tests.
    my $end = scalar @lines;
    for my $i (0 .. $#lines) {
      if ($lines[$i] =~ /^\s*#\[cfg\(test\)\]/) { $end = $i; last; }
    }
    # Paramètres déclarés &'"'"'static str (fragments littéraux).
    my %static;
    for my $i (0 .. $end - 1) {
      while ($lines[$i] =~ /(\w+)\s*:\s*&'"'"'static\s+str/g) { $static{$1} = 1; }
    }
    for my $i (0 .. $end - 1) {
      my $line = $lines[$i];
      my $n = $i + 1;
      if ($line =~ /execute_batch/) {
        print STDERR "ERREUR : $file:$n : execute_batch hors tests\n"; $status = 1;
      }
      if ($line =~ /\b(format|concat)!\s*\(/) {
        my $macro = $1;
        # Contenu exact de la macro : parenthèses équilibrées, chaînes
        # entre guillemets respectées, sur plusieurs lignes au besoin.
        my $rest = join("", @lines[$i .. $end - 1]);
        $rest =~ s/^.*?\b(?:format|concat)!\s*\(//s;
        my ($depth, $in_str, $esc, $text) = (1, 0, 0, "");
        for my $c (split //, $rest) {
          if ($in_str) {
            if ($esc) { $esc = 0 } elsif ($c eq "\\") { $esc = 1 } elsif ($c eq "\"") { $in_str = 0 }
          } elsif ($c eq "\"") { $in_str = 1 }
          elsif ($c eq "(") { $depth++ }
          elsif ($c eq ")") { $depth--; last if $depth == 0 }
          $text .= $c;
        }
        next unless $text =~ $sql;
        my $from = $i - 3 < 0 ? 0 : $i - 3;
        my $marked = grep { /sql-const/ } @lines[$from .. $i];
        if (!$marked) {
          print STDERR "ERREUR : $file:$n : SQL construit par $macro! sans commentaire sql-const\n";
          $status = 1;
        }
        # Interpolations du premier littéral de la macro.
        if ($text =~ /^\s*"((?:[^"\\]|\\.)*)"/s) {
          my $literal = $1;
          my $after = substr($text, $+[0]);
          if ($literal =~ /\{\d*(:[^}]*)?\}/ || $after =~ /\S/) {
            print STDERR "ERREUR : $file:$n : interpolation positionnelle ou argument dans du SQL\n";
            $status = 1;
          }
          while ($literal =~ /\{([A-Za-z_][A-Za-z0-9_]*)[^}]*\}/g) {
            my $name = $1;
            next if $name =~ /^[A-Z][A-Z0-9_]*$/ || $static{$name};
            print STDERR "ERREUR : $file:$n : « $name » interpolé dans du SQL (ni constante ni &'"'"'static str)\n";
            $status = 1;
          }
        }
      }
      if ($line =~ /push_str/ && $line =~ $sql) {
        print STDERR "ERREUR : $file:$n : SQL construit par push_str\n"; $status = 1;
      }
      if ($line =~ /"[^"]*$sql[^"]*"\s*\+|\+\s*"[^"]*$sql/) {
        print STDERR "ERREUR : $file:$n : SQL construit par concaténation\n"; $status = 1;
      }
    }
  }
  exit $status;
' $files || status=1

[ "$status" -eq 0 ] && echo "SQL : requêtes paramétrées uniquement (aucune interpolation de données)"
exit "$status"
