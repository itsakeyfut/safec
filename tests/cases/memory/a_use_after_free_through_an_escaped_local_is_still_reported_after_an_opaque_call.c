void *malloc(int n);
void free(void *p);
void stash(int **r);
void other(void);
int f(void) {
    int *p = malloc(4);
    stash(&p);
    other();
    free(p);
    *p = 1;
    return 0;
}
