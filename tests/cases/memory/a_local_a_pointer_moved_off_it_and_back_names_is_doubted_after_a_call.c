void fill(int **p);
int main(void) {
    int *slot;
    int **pp = &slot;
    pp = pp + 1;
    pp = pp - 1;
    fill(pp);
    int *q = slot;
    return q == 0;
}
