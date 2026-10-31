int main(void) {
    int x = 0;
    int *p = &x;
    int *slot;
    int **pp = &slot;
    pp = pp + 1;
    pp = pp - 1;
    *pp = p;
    int *q = slot;
    return q == &x;
}
