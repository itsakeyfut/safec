#define X 1
#define F(a) (a+1)
#define G F
#define OPEN F(

#if (defined X) && F(0) == 1 && defined(X)
int d;
#endif
