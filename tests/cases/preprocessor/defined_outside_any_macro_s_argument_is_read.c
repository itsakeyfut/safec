#define X 1
#define F(a) (a+1)
#define G F
#define OPEN F(
#define CALL(m) m
#define NOT !
#define ALSO_OPEN OPEN
#define SELF SELF
#define F2(a) F(
#define VIA_CALL F2(0)

#if (defined X) && F(0) == 1 && defined(X)
int d;
#endif

#if NOT (defined X)
#else
int f;
#endif

#if OPEN 0) == 1 && defined X
int g;
#endif
