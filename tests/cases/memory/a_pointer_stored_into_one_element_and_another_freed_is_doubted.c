void *malloc(int n);
void free(void *p);
int main(void) {
    int *m[2];
    int *p = malloc(4);
    if (p == 0) { return 1; }
    int *r = malloc(4);
    if (r == 0) { free(p); return 1; }
    m[0] = p;
    m[1] = r;
    free(r);
    int *q = m[0];
    if (q != 0) { int v = *q; free(p); return v; }
    free(p);
    return 0;
}
