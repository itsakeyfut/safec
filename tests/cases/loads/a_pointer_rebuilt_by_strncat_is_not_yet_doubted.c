// strncat is not read as a copy, since its length bounds what it reads of
// the source, so a pointer it rebuilds is not doubted: a silence the
// documents name, held here so that closing it renames this case (#691).
void *malloc(int n); void *calloc(int n, int s); void free(void *p);
char *strncat(char *d, char *s, int n);
int main(void) {
    int *p = malloc(4);
    if (!p) return 1;
    int **a = calloc(1, 8); int **b = calloc(1, 8);
    if (!a || !b) return 1;
    *a = p;
    void *va = a; void *vb = b;
    char *ca = va; char *cb = vb;
    strncat(cb, ca, 8);
    free(p);
    int *q = *b;
    if (!q) return 0;
    int r = *q;
    return r;
}
