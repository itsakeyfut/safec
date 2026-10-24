void *malloc(int n);
void release_ref(int **pp);
int use2(int **pp);

int f(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int *c = malloc(4);
    if (c == 0) {
        return 0;
    }
    int *b = c;
    release_ref(&b);
    use2(&b);
    return use2(&a);
}
