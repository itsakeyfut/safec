void free(void *p);
void stash(int *p);
int *get(void);

int g(void) {
    int x;
    stash(&x);
    int *r = get();
    free(r);
    return 0;
}
