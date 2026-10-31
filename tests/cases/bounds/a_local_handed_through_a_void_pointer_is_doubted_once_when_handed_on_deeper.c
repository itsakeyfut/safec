void put_any(void *p);
void use2(int **p);
void run(void) {
    int **v = 0;
    put_any(&v);
    use2(v);
}
