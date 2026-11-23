enum Unsigned { A, B };
enum Signed { M = -1, N };
int below(enum Unsigned u, enum Signed s) {
    return (u - 1 < 0) + (s - 1 < 0);
}
