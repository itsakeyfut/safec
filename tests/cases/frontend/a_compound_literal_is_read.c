struct S {
    int a, b;
};
int *g = (int[]){1, 2};
int f(void) {
    return ((struct S){.b = 1}).b + sizeof (int[]){1, 2, 3};
}
