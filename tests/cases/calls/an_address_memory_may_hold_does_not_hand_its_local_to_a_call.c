void *malloc(int n);
void stash(int ***t);
void grow_all(void);
int use2(int **pp);

int f(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int *b = a;
    int ***t = malloc(8);
    if (t == 0) {
        return 0;
    }
    *t = &a;
    *t = &b;
    stash(t);
    grow_all();
    return use2(&a);
}
