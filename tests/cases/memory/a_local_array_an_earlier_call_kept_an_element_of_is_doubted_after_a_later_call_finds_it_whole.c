void keep2(int ***pe);
void tick(void);
int main(void) {
    int *m[2];
    int **e = m + 1;
    keep2(&e);
    e = m;
    tick();
    int *r = m[0];
    return r != 0;
}
