void *malloc(int n);
void stash(int ***t);
void release_ref(int **pp);
int use2(int **pp);

int f(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int ***t = malloc(8);
    if (t == 0) {
        return 0;
    }
    *t = &a;
    *t = 0;
    stash(t);
    int *b = a;
    release_ref(&b);
    return use2(&a);
}
