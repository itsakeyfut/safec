void *malloc(int n);
void free(void *p);
void stash(int **r);
int f(void) {
    int *p = malloc(4);
    int *r = p;
    stash(&p);
    free(r);
    free(p);
    *r = 1;
    return 0;
}
