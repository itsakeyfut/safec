void *malloc(int n);
void free(void *p);
void get(int **pp);
int f(int c) {
    int *u;
    get(&u);
    int *p = malloc(4);
    if (!p) return 0;
    *p = 1;
    if (c) { free(p); p = 0; }
    if (!p) return 0;
    int r = *p;
    free(p);
    return r;
}
