void *malloc(int n);
void fill(int ***b);
int ***mk(void) {
    return malloc(8);
}
int f(int c) {
    int *x;
    int ***a = mk();
    int ***b = mk();
    if (!a) return 0;
    if (!b) return 0;
    *a = &x;
    fill(b);
    return x != 0;
}
