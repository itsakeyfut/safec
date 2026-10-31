int main(void) {
    int x = 0;
    int *z;
    int **pp = &z;
    pp = pp + 1;
    *pp = &x;
    int *q = z;
    return q == &x;
}
