void *malloc(int n);
void stash(int **pp);
void release_ref(int **pp);
int use2(int **pp);

int f(int c) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int **pa = &a;
    if (c) {
        stash(pa);
    }
    int *b = a;
    release_ref(&b);
    return use2(pa);
}
