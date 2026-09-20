void *malloc(int n);
void free(void *p);
void stash(int **r);
void other(void);
int f(void) {
    int *p = malloc(4);
    int *q = p;
    stash(&p);
    other();
    free(p);
    free(q);
    return 0;
}
