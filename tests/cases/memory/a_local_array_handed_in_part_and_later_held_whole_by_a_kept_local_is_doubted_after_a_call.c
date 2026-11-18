void fill(int **s);
void stash(int ***p);
void tick(void);
int main(void) {
    int *m[2];
    int **h;
    fill(m + 1);
    stash(&h);
    h = m;
    tick();
    int *r = m[0];
    return r != 0;
}
