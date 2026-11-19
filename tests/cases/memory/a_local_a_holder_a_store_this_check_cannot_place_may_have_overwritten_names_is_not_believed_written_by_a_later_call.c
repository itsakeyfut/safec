void stash(int ***pp, int ****box); void tick(void); void fill(int **p);
int main(void) {
    int *a;
    int *b = 0;
    int **h = &b;
    int ***cell = 0;
    stash(&h, &cell);
    int ***t = cell;
    h = &a;
    if (t) *t = &b;
    tick();
    return a != 0;
}
