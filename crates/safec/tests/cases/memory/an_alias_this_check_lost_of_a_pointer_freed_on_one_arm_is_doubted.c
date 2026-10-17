void *malloc(int n);
void free(void *p);
void stash(int **pp);
int release_all(void);
int f(int c) {
    int *p = malloc(4);
    if (!p) return 0;
    *p = 1;
    int *b = p;
    stash(&b);
    release_all();
    if (c) { free(p); p = 0; }
    if (!b) return 0;
    return *b;
}
