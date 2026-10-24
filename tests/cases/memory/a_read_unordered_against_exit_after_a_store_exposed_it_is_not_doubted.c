void exit(int status);

int f(int *a, int **out) {
    return (a[0] + (*out = a, 0)) + (exit(1), 0);
}
