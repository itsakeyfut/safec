void use(int ***p);
void run(void ****m) {
    void *pm = m;
    int ****pi = pm;
    if (pi != 0) {
        use(*pi);
    }
}
