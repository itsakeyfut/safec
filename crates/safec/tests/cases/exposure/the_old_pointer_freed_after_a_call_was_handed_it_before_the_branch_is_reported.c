void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);
void g(void *p);
int main(void) {
    int *p = malloc(4);
    if (p == 0) return 0;
    int *q = realloc(p, 8);
    g(p);
    if (q == 0) { free(p); return 0; }
    free(q);
    return 0;
}
