void put(int ****p);
void use(int ***p);
void run(void) {
    int ***v = 0;
    put(&v);
    use(v);
}
