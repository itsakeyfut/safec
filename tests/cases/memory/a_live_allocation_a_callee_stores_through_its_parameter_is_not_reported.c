void *malloc(int n);
void put(int ** _Nonnull pp) { int *x = malloc(4); if (x == 0) { return; } *x = 1; *pp = x; }
int main(void) {
    int *m[1];
    put(m);
    int *q = m[0];
    if (q == 0) { return 1; }
    return *q;
}
